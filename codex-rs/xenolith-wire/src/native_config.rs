// Copyright (c) 2026 CodeXe contributors. Licensed under Apache-2.0.

//! The engine resolves its configuration. This module only validates its local
//! discovery response; it never reads the engine's configuration file.

use serde::Deserialize;
use std::io;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

const OUTPUT_LIMIT: usize = 64 * 1024;
pub(crate) const INSPECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Deserialize)]
pub(crate) struct Endpoint {
    pub transport: String,
    pub path: PathBuf,
}

#[derive(Deserialize)]
pub(crate) struct Inspection {
    pub schema_version: u32,
    pub config_path: PathBuf,
    pub config_present: bool,
    pub endpoint: Endpoint,
    pub engine_lock: PathBuf,
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub log_path: PathBuf,
}

impl Inspection {
    fn parse(bytes: &[u8]) -> io::Result<Self> {
        let info: Self = serde_json::from_slice(bytes).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid Xenolith config inspect JSON: {error}"),
            )
        })?;
        if info.schema_version != 1 || info.endpoint.transport != "unix" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported Xenolith config inspect schema or transport",
            ));
        }
        for (name, path) in [
            ("config_path", &info.config_path),
            ("endpoint.path", &info.endpoint.path),
            ("engine_lock", &info.engine_lock),
            ("state_dir", &info.state_dir),
            ("cache_dir", &info.cache_dir),
            ("log_path", &info.log_path),
        ] {
            if !path.is_absolute() || path.to_str().is_none_or(|p| p.contains('\0')) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Xenolith config inspect {name} must be an absolute UTF-8 path"),
                ));
            }
        }
        Ok(info)
    }
}

fn nonblocking(pipe: &impl AsRawFd) -> io::Result<()> {
    let fd = pipe.as_raw_fd();
    // SAFETY: these operations only change flags on this live pipe descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn drain(pipe: &mut impl Read, buffer: &mut Vec<u8>) -> io::Result<()> {
    let mut chunk = [0; 4096];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                if buffer.len() + count > OUTPUT_LIMIT {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Xenolith config inspect output exceeds 64 KiB",
                    ));
                }
                buffer.extend_from_slice(&chunk[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

/// Observe termination without reaping the leader. Keeping its PID reserved
/// lets failure cleanup safely signal the owned process group, including helpers
/// whose parent exited before producing its invalid response.
fn observed_exit(pid: u32) -> io::Result<Option<(bool, i32)>> {
    // SAFETY: waitid initializes this siginfo structure. WNOWAIT preserves our
    // Child ownership; child.wait() below performs the sole final reap.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: these fields are provided by successful waitid for P_PID.
    if unsafe { info.si_pid() } == 0 {
        return Ok(None);
    }
    Ok(Some((info.si_code == libc::CLD_EXITED, unsafe {
        info.si_status()
    })))
}

/// None means only the known pre-config Xenolith usage response. All other
/// failures, including malformed configuration and schemas, remain errors.
pub(crate) fn inspect(
    mut command: Command,
    cancelled: &AtomicBool,
    timeout: Duration,
) -> io::Result<Option<Inspection>> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Xenolith startup cancelled",
        ));
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let result = (|| {
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("missing inspect stdout"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("missing inspect stderr"))?;
        nonblocking(&stdout)?;
        nonblocking(&stderr)?;
        let deadline = Instant::now() + timeout;
        let mut out = Vec::new();
        let mut err = Vec::new();
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Xenolith startup cancelled",
                ));
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Xenolith config inspect timed out",
                ));
            }
            drain(&mut stdout, &mut out)?;
            drain(&mut stderr, &mut err)?;
            if let Some((exited, code)) = observed_exit(child.id())? {
                drain(&mut stdout, &mut out)?;
                drain(&mut stderr, &mut err)?;
                if exited && code == 0 {
                    return Inspection::parse(&out).map(Some);
                }
                let diagnostic = String::from_utf8_lossy(&err);
                if exited
                    && code == 1
                    && out.is_empty()
                    && diagnostic.starts_with("Usage: xenolith <command> <model.gguf> [options]\n")
                {
                    return Ok(None);
                }
                return Err(io::Error::other(format!(
                    "Xenolith config inspect failed ({} {code}): {}",
                    if exited { "exit code" } else { "signal" },
                    diagnostic.trim(),
                )));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    if result.is_err() {
        // SAFETY: this is the new process group created for our inspection
        // child. Kill only that group, including a helper it might have started.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        let _ = child.kill();
    }
    let reaped = child.wait();
    match result {
        Ok(info) => {
            reaped?;
            Ok(info)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response() -> serde_json::Value {
        json!({
            "schema_version": 1,
            "config_path": "/home/user/.config/xenolith/config.json",
            "config_present": true,
            "endpoint": {"transport": "unix", "path": "/run/user/1/xenolith/wire.sock"},
            "engine_lock": "/home/user/.local/state/xenolith/engine.lock",
            "state_dir": "/home/user/.local/state/xenolith",
            "cache_dir": "/home/user/.cache/xenolith/kv",
            "log_path": "/home/user/.local/state/xenolith/service.log",
        })
    }

    fn python(script: &str) -> Command {
        let mut command = Command::new("python3");
        command.args(["-c", script]);
        command
    }

    #[test]
    fn validates_version_transport_paths_and_required_types() {
        let original = response();
        let mut additive = original.clone();
        additive["future_field"] = json!("ignored");
        assert!(Inspection::parse(&serde_json::to_vec(&additive).unwrap()).is_ok());
        let mut cases = Vec::new();
        let mut schema = original.clone();
        schema["schema_version"] = json!(2);
        cases.push(schema);
        let mut transport = original.clone();
        transport["endpoint"]["transport"] = json!("tcp");
        cases.push(transport);
        let mut relative = original.clone();
        relative["endpoint"]["path"] = json!("relative.sock");
        cases.push(relative);
        let mut nul = original.clone();
        nul["log_path"] = json!("/tmp/bad\0path");
        cases.push(nul);
        let mut missing = original.clone();
        missing.as_object_mut().unwrap().remove("engine_lock");
        cases.push(missing);
        let mut wrong_type = original.clone();
        wrong_type["config_present"] = json!("yes");
        cases.push(wrong_type);
        for case in cases {
            assert!(
                Inspection::parse(&serde_json::to_vec(&case).unwrap()).is_err(),
                "{case}"
            );
        }
        assert!(Inspection::parse(b"{}\n{}").is_err());
    }

    #[test]
    fn runs_bounded_inspection_without_a_shell_and_returns_no_model_parameters() {
        let script = format!("print({:?})", serde_json::to_string(&response()).unwrap());
        let info = inspect(python(&script), &AtomicBool::new(false), INSPECT_TIMEOUT)
            .unwrap()
            .unwrap();
        assert_eq!(
            info.endpoint.path,
            PathBuf::from("/run/user/1/xenolith/wire.sock")
        );
    }

    #[test]
    fn only_known_legacy_usage_allows_fallback() {
        let legacy = "import sys; print('Usage: xenolith <command> <model.gguf> [options]', file=sys.stderr); sys.exit(1)";
        assert!(
            inspect(python(legacy), &AtomicBool::new(false), INSPECT_TIMEOUT)
                .unwrap()
                .is_none()
        );
        for script in [
            "import sys; print('invalid native configuration', file=sys.stderr); sys.exit(1)",
            "import sys; print('Usage: some other program', file=sys.stderr); sys.exit(1)",
            "print('not JSON')",
        ] {
            assert!(inspect(python(script), &AtomicBool::new(false), INSPECT_TIMEOUT).is_err());
        }
    }

    #[test]
    fn caps_stdout_and_stderr_and_bounds_inspection_time() {
        for script in [
            "print('x' * 70000)",
            "import sys; sys.stderr.write('x' * 70000)",
        ] {
            let error = inspect(python(script), &AtomicBool::new(false), INSPECT_TIMEOUT)
                .err()
                .unwrap();
            assert!(error.to_string().contains("64 KiB"));
        }
        let start = Instant::now();
        let error = inspect(
            python("import time; time.sleep(30)"),
            &AtomicBool::new(false),
            Duration::from_millis(50),
        )
        .err()
        .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn invalid_response_kills_helpers_after_the_leader_exits() {
        let pid_file =
            std::env::temp_dir().join(format!("xenolith-inspect-helper-{}", std::process::id()));
        let mut command = python(
            "import subprocess,sys; child=subprocess.Popen(['sleep','30']); open(sys.argv[1],'w').write(str(child.pid)); print('invalid JSON')",
        );
        command.arg(&pid_file);
        let error = inspect(command, &AtomicBool::new(false), INSPECT_TIMEOUT)
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
        let _ = std::fs::remove_file(pid_file);
        // A killed orphan may briefly remain a zombie until the system reaps
        // it. Both zombie and missing states prove it cannot continue running.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
            if stat
                .as_ref()
                .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
                || stat.as_ref().is_ok_and(|value| {
                    value
                        .rsplit_once(") ")
                        .is_some_and(|(_, tail)| tail.starts_with('Z'))
                })
            {
                break;
            }
            if Instant::now() >= deadline {
                // This PID was allocated by our fixture, not an unrelated
                // process. Stop it before reporting the regression failure.
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
                panic!("inspection helper survived failed discovery");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn cancellation_interrupts_inspect_before_full_timeout() {
        let cancelled = std::sync::Arc::new(AtomicBool::new(false));
        let signal = cancelled.clone();
        let worker = std::thread::spawn(move || {
            inspect(
                python("import time; time.sleep(30)"),
                &signal,
                INSPECT_TIMEOUT,
            )
        });
        std::thread::sleep(Duration::from_millis(40));
        let start = Instant::now();
        cancelled.store(true, Ordering::Release);
        let error = worker.join().unwrap().err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
