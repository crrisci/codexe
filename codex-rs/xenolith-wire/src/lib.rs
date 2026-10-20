// Xenolith native integration: added or modified by Francesco Pio Crisci, 2026.
// Licensed under Apache-2.0; upstream copyright and notices are retained.

//! Bounded Xenolith JSONL framing over Unix sockets.
//!
//! This transport does not parse JSON, invent terminal events, retry requests,
//! or own engine sessions. Protocol validation and durable bindings belong to
//! the native backend added in the next step.

#![cfg(unix)]

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

struct CancellationWatch {
    finished: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Drop for CancellationWatch {
    fn drop(&mut self) {
        self.finished.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub mod backend;
mod images;
pub mod protocol;
pub mod service;

pub struct WireConnection {
    reader: BufReader<UnixStream>,
    max_frame: usize,
    poisoned: bool,
    generating: bool,
    cancellation: Option<CancellationWatch>,
}

impl WireConnection {
    /// Interrupt blocked reads even on engines without progress events.
    /// Cancellation remains best effort; uncertain mutations are not replayed.
    pub fn watch_cancellation(&mut self, cancelled: Arc<AtomicBool>) -> io::Result<()> {
        let mut socket = self.reader.get_ref().try_clone()?;
        let finished = Arc::new(AtomicBool::new(false));
        let stop = finished.clone();
        let worker = std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                if cancelled.load(Ordering::Acquire) {
                    let _ = socket.set_write_timeout(Some(Duration::from_millis(100)));
                    let _ = socket.write_all(b"{\"op\":\"cancel\"}\n");
                    let _ = socket.shutdown(std::net::Shutdown::Both);
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        self.cancellation = Some(CancellationWatch {
            finished,
            worker: Some(worker),
        });
        Ok(())
    }
    pub fn generation_started(&mut self) {
        self.generating = true;
    }
    pub fn generation_finished(&mut self) {
        self.generating = false;
    }

    pub fn set_max_frame(&mut self, max_frame: usize) -> io::Result<()> {
        if !(1..=96 * 1024 * 1024).contains(&max_frame) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid max_frame",
            ));
        }
        self.max_frame = max_frame;
        Ok(())
    }

    /// `max_frame` is a byte bound excluding the terminating LF.
    /// Read/write timeouts prevent indefinitely blocking on a stalled peer.
    pub fn connect(path: &Path, max_frame: usize, timeout: Duration) -> io::Result<Self> {
        Self::from_stream(UnixStream::connect(path)?, max_frame, timeout)
    }

    fn from_stream(stream: UnixStream, max_frame: usize, timeout: Duration) -> io::Result<Self> {
        if max_frame == 0 || max_frame == usize::MAX || timeout.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid wire limits",
            ));
        }
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        Ok(Self {
            reader: BufReader::new(stream),
            max_frame,
            poisoned: false,
            generating: false,
            cancellation: None,
        })
    }

    fn check_live(&self) -> io::Result<()> {
        if self.poisoned {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "wire connection is unusable",
            ));
        }
        Ok(())
    }

    /// Send one already-serialized JSON frame. Never retry a partial write:
    /// whether the engine accepted a mutation is then unknown.
    pub fn send_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        self.check_live()?;
        if frame.is_empty() || frame.len() > self.max_frame || frame.contains(&b'\n') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid wire frame",
            ));
        }
        let result = (|| {
            self.reader.get_mut().write_all(frame)?;
            self.reader.get_mut().write_all(b"\n")
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    /// Read one UTF-8 frame without the LF; EOF is an error, never `done`.
    /// All receive failures poison the connection, including partial timeout.
    pub fn receive_frame(&mut self) -> io::Result<String> {
        self.check_live()?;
        let result = (|| {
            let mut bytes = Vec::new();
            (&mut self.reader)
                .take(self.max_frame as u64 + 1)
                .read_until(b'\n', &mut bytes)?;
            if bytes.last() != Some(&b'\n') {
                let kind = if bytes.len() > self.max_frame {
                    io::ErrorKind::InvalidData
                } else {
                    io::ErrorKind::UnexpectedEof
                };
                return Err(io::Error::new(kind, "oversized or unterminated wire frame"));
            }
            bytes.pop();
            if bytes.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "empty wire frame",
                ));
            }
            String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
}

impl Drop for WireConnection {
    fn drop(&mut self) {
        if self.generating {
            // Best effort on failure or dropped consumer. Never wait indefinitely
            // and never turn a missing cancellation ACK into successful completion.
            let stream = self.reader.get_mut();
            let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
            let _ = stream.write_all(b"{\"op\":\"cancel\"}\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Shutdown;

    fn pair(limit: usize) -> (WireConnection, UnixStream) {
        let (client, server) = UnixStream::pair().unwrap();
        (
            WireConnection::from_stream(client, limit, Duration::from_secs(1)).unwrap(),
            server,
        )
    }

    #[test]
    fn sends_describe_without_http() {
        let (mut client, server) = pair(64);
        client.send_frame(br#"{"op":"describe"}"#).unwrap();
        let mut frame = String::new();
        BufReader::new(server).read_line(&mut frame).unwrap();
        assert_eq!(frame, "{\"op\":\"describe\"}\n");
    }

    #[test]
    fn preserves_multiple_frames() {
        let (mut client, mut server) = pair(64);
        server
            .write_all(b"{\"event\":\"start\"}\n{\"event\":\"done\"}\n")
            .unwrap();
        assert_eq!(client.receive_frame().unwrap(), "{\"event\":\"start\"}");
        assert_eq!(client.receive_frame().unwrap(), "{\"event\":\"done\"}");
    }

    #[test]
    fn accepts_exact_byte_limit() {
        let (mut client, mut server) = pair(2);
        server.write_all(b"{}\n").unwrap();
        assert_eq!(client.receive_frame().unwrap(), "{}");
    }

    #[test]
    fn rejects_oversized_input_and_poisoned_reuse() {
        let (mut client, mut server) = pair(2);
        server.write_all(b"xxx\n{}\n").unwrap();
        assert_eq!(
            client.receive_frame().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            client.receive_frame().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn eof_does_not_complete_generation() {
        let (mut client, mut server) = pair(64);
        server.write_all(b"{}").unwrap();
        server.shutdown(Shutdown::Write).unwrap();
        assert_eq!(
            client.receive_frame().unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn rejects_empty_and_invalid_utf8_frames() {
        for bytes in [b"\n".as_slice(), b"\xff\n".as_slice()] {
            let (mut client, mut server) = pair(64);
            server.write_all(bytes).unwrap();
            assert_eq!(
                client.receive_frame().unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn refuses_outgoing_frame_injection() {
        let (mut client, _server) = pair(64);
        assert_eq!(
            client.send_frame(b"{}\n{}").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        client.send_frame(b"{}").unwrap();
    }

    #[test]
    fn stalled_peer_times_out_and_poisoned_reuse_fails() {
        let (client, _server) = UnixStream::pair().unwrap();
        let mut client =
            WireConnection::from_stream(client, 64, Duration::from_millis(20)).unwrap();
        assert!(matches!(
            client.receive_frame().unwrap_err().kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ));
        assert_eq!(
            client.send_frame(b"{}").unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }
}
