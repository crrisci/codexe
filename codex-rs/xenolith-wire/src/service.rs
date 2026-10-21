// Xenolith native integration: added or modified by Francesco Pio Crisci, 2026.
// Licensed under Apache-2.0; upstream copyright and notices are retained.

//! Shared Unix service discovery and optional startup, independent of native
//! conversation bindings. A successfully started service outlives discovery and
//! its client; the engine's idle shutdown policy controls its lifetime.

use crate::WireConnection;
use crate::protocol::Describe;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs::File;
use std::fs::OpenOptions;
use std::fs::{self};
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::{self};
use std::os::fd::AsRawFd;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

const BOOTSTRAP_MAX_FRAME: usize = 4 * 1024 * 1024;
const LOG_TAIL_BYTES: u64 = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServicePhase {
    Connecting,
    Starting,
    Waiting,
    Ready,
}

#[derive(Clone, Debug)]
pub struct ServiceSettings {
    pub socket: PathBuf,
    pub binary: OsString,
    pub model: Option<PathBuf>,
    pub mmproj: Option<PathBuf>,
    pub engine_state_dir: PathBuf,
    pub engine_cache_dir: PathBuf,
    /// The engine's global lock, independent of `--state` and socket overrides.
    pub engine_lock: PathBuf,
    pub context: Option<u64>,
    pub cache_budget: Option<u64>,
    pub idle_shutdown_minutes: f64,
    pub spawn: bool,
    pub startup_timeout: Duration,
    pub request_timeout: Duration,
}

fn input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn absolute(path: PathBuf, source: &str) -> io::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(input(format!("{source} must be an absolute Unix path")));
    }
    Ok(path)
}

impl ServiceSettings {
    /// Environment projector takes precedence over the provider's config.toml.
    pub fn with_mmproj(mut self, configured: Option<&Path>) -> io::Result<Self> {
        if self.mmproj.is_none() {
            self.mmproj = configured
                .map(|path| absolute(path.to_owned(), "mmproj"))
                .transpose()?;
        }
        Ok(self)
    }

    /// Provider `unix:///absolute/path` takes precedence over XENOLITH_SOCKET.
    /// XENOLITH_STATE_DIR belongs to native Codex bindings and is intentionally
    /// not read here. Engine data and KV cache use their own explicit overrides.
    pub fn from_env(base_url: Option<&str>) -> io::Result<Self> {
        Self::from_lookup(base_url, |name| std::env::var_os(name))
    }

    fn from_lookup(
        base_url: Option<&str>,
        env: impl Fn(&str) -> Option<OsString>,
    ) -> io::Result<Self> {
        let home = || -> io::Result<PathBuf> {
            absolute(
                env("HOME").map(PathBuf::from).ok_or_else(|| {
                    input("set HOME or absolute XDG_STATE_HOME and XDG_CACHE_HOME")
                })?,
                "HOME",
            )
        };
        let xdg = |name: &str| env(name).map(PathBuf::from).filter(|p| p.is_absolute());
        let default_state = match xdg("XDG_STATE_HOME") {
            Some(path) => path.join("xenolith"),
            None => home()?.join(".local/state/xenolith"),
        };
        let default_cache = match xdg("XDG_CACHE_HOME") {
            Some(path) => path.join("xenolith/kv"),
            None => home()?.join(".cache/xenolith/kv"),
        };
        let engine_state_dir = absolute(
            env("XENOLITH_ENGINE_STATE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| default_state.clone()),
            "XENOLITH_ENGINE_STATE_DIR",
        )?;
        let engine_cache_dir = absolute(
            env("XENOLITH_ENGINE_CACHE_DIR")
                .map(PathBuf::from)
                .unwrap_or(default_cache),
            "XENOLITH_ENGINE_CACHE_DIR",
        )?;
        let socket = if let Some(url) = base_url {
            PathBuf::from(
                url.strip_prefix("unix://")
                    .ok_or_else(|| input("Xenolith base_url must use unix:///absolute/path"))?,
            )
        } else if let Some(path) = env("XENOLITH_SOCKET").filter(|s| !s.is_empty()) {
            PathBuf::from(path)
        } else if let Some(path) = xdg("XDG_RUNTIME_DIR") {
            path.join("xenolith/wire.sock")
        } else {
            engine_state_dir.join("wire.sock")
        };
        let socket = absolute(socket, "Xenolith socket")?;
        if socket.to_str().is_none() {
            return Err(input("Xenolith socket path must be valid UTF-8"));
        }
        let context = env("XENOLITH_CONTEXT")
            .map(|value| {
                value
                    .to_str()
                    .filter(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()))
                    .and_then(|s| s.parse::<u64>().ok())
                    .filter(|n| *n > 0)
                    .ok_or_else(|| input("XENOLITH_CONTEXT must be a positive integer token count"))
            })
            .transpose()?;
        let cache_budget = env("XENOLITH_CACHE_BUDGET")
            .map(|value| parse_cache_budget(&value))
            .transpose()?;
        let idle_shutdown_minutes = env("XENOLITH_IDLE_SHUTDOWN")
            .map(|value| {
                value
                    .to_str()
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|n| n.is_finite() && *n > 0.0)
                    .ok_or_else(|| input("XENOLITH_IDLE_SHUTDOWN must be positive finite minutes"))
            })
            .transpose()?
            .unwrap_or(30.0);
        let model = env("XENOLITH_MODEL")
            .filter(|s| !s.is_empty())
            .map(|p| absolute(PathBuf::from(p), "XENOLITH_MODEL"))
            .transpose()?;
        Ok(Self {
            socket,
            binary: env("XENOLITH_BIN")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "xenolith".into()),
            model,
            mmproj: env("XENOLITH_MMPROJ")
                .filter(|s| !s.is_empty())
                .map(|p| absolute(PathBuf::from(p), "XENOLITH_MMPROJ"))
                .transpose()?,
            engine_state_dir,
            engine_cache_dir,
            engine_lock: default_state.join("engine.lock"),
            context,
            cache_budget,
            idle_shutdown_minutes,
            spawn: env("XENOLITH_NO_SPAWN").as_deref() != Some(OsStr::new("1")),
            startup_timeout: Duration::from_secs(120),
            request_timeout: Duration::from_secs(5),
        })
    }
}

fn parse_cache_budget(value: &OsStr) -> io::Result<u64> {
    let bad = || input("XENOLITH_CACHE_BUDGET must be a uint64 byte count or size such as 4GiB");
    let text = value.to_str().ok_or_else(bad)?;
    let split = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let count: u64 = text[..split].parse().map_err(|_| bad())?;
    let multiplier = match &text[split..] {
        "" | "B" => 1,
        "kB" => 1000,
        "MB" => 1_000_000,
        "GB" => 1_000_000_000,
        "KiB" => 1024,
        "MiB" => 1024 * 1024,
        "GiB" => 1024 * 1024 * 1024,
        _ => return Err(bad()),
    };
    count.checked_mul(multiplier).ok_or_else(bad)
}

/// Connect-only discovery. Uses the same validated describe/max_frame contract
/// as inference, and never allocates model weights or modifies engine state.
pub fn describe(settings: &ServiceSettings) -> io::Result<Describe> {
    let mut wire =
        connect(settings, settings.request_timeout).map_err(|e| diagnostic(settings, e))?;
    Describe::discover(&mut wire).map_err(|e| diagnostic(settings, e))
}

fn connect(settings: &ServiceSettings, timeout: Duration) -> io::Result<WireConnection> {
    WireConnection::connect(&settings.socket, BOOTSTRAP_MAX_FRAME, timeout)
}

fn diagnostic(settings: &ServiceSettings, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("Xenolith service at {}: {error}", settings.socket.display()),
    )
}

fn private_directory(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

fn unavailable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

/// Connect first; only an absent/refused socket permits optional startup.
/// The exact engine weight lock is handed to the child, preventing another
/// client (including another socket or XenoPi) from allocating competing weights.
pub fn ensure_service(settings: &ServiceSettings) -> io::Result<Describe> {
    ensure_service_with_status(settings, |_| {})
}

/// Startup phases describe observed lifecycle transitions, never load percentages.
pub fn ensure_service_with_status(
    settings: &ServiceSettings,
    on_status: impl FnMut(ServicePhase),
) -> io::Result<Describe> {
    ensure_service_cancellable(settings, Arc::new(AtomicBool::new(false)), on_status)
}

fn startup_cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Xenolith startup cancelled")
}

/// Cancellation closes only this discovery connection, without sending a native
/// cancel operation to a service that may belong to another client.
fn discover_cancellable(
    wire: &mut WireConnection,
    cancelled: &Arc<AtomicBool>,
) -> io::Result<Describe> {
    let socket = wire.reader.get_ref().try_clone()?;
    let finished = Arc::new(AtomicBool::new(false));
    let stop = finished.clone();
    let signal = cancelled.clone();
    let watcher = std::thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            if signal.load(Ordering::Acquire) {
                let _ = socket.shutdown(std::net::Shutdown::Both);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    let result = Describe::discover(wire);
    finished.store(true, Ordering::Release);
    let _ = watcher.join();
    if result.is_err() && cancelled.load(Ordering::Acquire) {
        return Err(startup_cancelled());
    }
    result
}

/// An interrupted startup reaps only its own not-yet-ready child. A service that
/// has completed describe retains its detached lifetime, including on cancellation.
pub fn ensure_service_cancellable(
    settings: &ServiceSettings,
    cancelled: Arc<AtomicBool>,
    mut on_status: impl FnMut(ServicePhase),
) -> io::Result<Describe> {
    if cancelled.load(Ordering::Acquire) {
        return Err(startup_cancelled());
    }
    on_status(ServicePhase::Connecting);
    match connect(settings, settings.request_timeout) {
        Ok(mut wire) => {
            let info =
                discover_cancellable(&mut wire, &cancelled).map_err(|e| diagnostic(settings, e))?;
            on_status(ServicePhase::Ready);
            return Ok(info);
        }
        Err(error) if unavailable(&error) => {
            if !settings.spawn {
                return Err(diagnostic(
                    settings,
                    io::Error::new(
                        error.kind(),
                        format!("{error}; automatic startup disabled by XENOLITH_NO_SPAWN"),
                    ),
                ));
            }
        }
        Err(error) => return Err(diagnostic(settings, error)),
    }
    if settings.startup_timeout.is_zero() || settings.request_timeout.is_zero() {
        return Err(input("Xenolith service timeouts must be positive"));
    }
    let model = settings.model.as_ref().ok_or_else(|| diagnostic(settings, input(
        "no service reachable; set XENOLITH_MODEL to an absolute model path for automatic startup, or start xenolith serve separately",
    )))?;
    let lock_parent = settings
        .engine_lock
        .parent()
        .ok_or_else(|| input("missing engine lock directory"))?;
    private_directory(lock_parent)?;
    let mut lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&settings.engine_lock)?;
    let deadline = Instant::now() + settings.startup_timeout;
    let log_path = settings.engine_state_dir.join("service.log");
    let mut child: Option<Child> = None;
    let mut waiting_reported = false;
    let last_error;
    loop {
        if cancelled.load(Ordering::Acquire) {
            last_error = startup_cancelled();
            break;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            last_error = io::Error::new(
                io::ErrorKind::TimedOut,
                "startup readiness deadline exceeded",
            );
            break;
        }
        match connect(settings, settings.request_timeout.min(remaining)) {
            Ok(mut wire) => match discover_cancellable(&mut wire, &cancelled) {
                Ok(info) => {
                    if let Some(mut owned) = child.take() {
                        // Drop never kills the detached service; reap it if this
                        // client remains alive until its idle shutdown.
                        std::thread::spawn(move || {
                            let _ = owned.wait();
                        });
                    }
                    on_status(ServicePhase::Ready);
                    return Ok(info);
                }
                Err(error) => {
                    last_error = error;
                    break;
                }
            },
            Err(error) if unavailable(&error) => {}
            Err(error) => {
                last_error = error;
                break;
            }
        }
        if let Some(owned) = child.as_mut() {
            if let Some(status) = owned.try_wait()? {
                // The engine lock may have been won by an independently
                // launched service; one final connection is safe and read-only.
                if let Ok(mut wire) = connect(settings, settings.request_timeout.min(remaining))
                    && let Ok(info) = discover_cancellable(&mut wire, &cancelled)
                {
                    on_status(ServicePhase::Ready);
                    return Ok(info);
                }
                last_error = io::Error::other(format!("spawned xenolith exited with {status}"));
                break;
            }
        } else {
            // SAFETY: the lock descriptor is live and flock only changes its lock.
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                // Recheck under the lock before spawning. Never unlink a socket.
                if let Ok(mut wire) = connect(settings, settings.request_timeout.min(remaining)) {
                    let info = discover_cancellable(&mut wire, &cancelled)
                        .map_err(|e| diagnostic(settings, e))?;
                    on_status(ServicePhase::Ready);
                    return Ok(info);
                }
                private_directory(
                    settings
                        .socket
                        .parent()
                        .ok_or_else(|| input("missing socket parent"))?,
                )?;
                private_directory(&settings.engine_state_dir)?;
                private_directory(&settings.engine_cache_dir)?;
                if cancelled.load(Ordering::Acquire) {
                    last_error = startup_cancelled();
                    break;
                }
                child = Some(
                    spawn_service(settings, model, &lock, &log_path).map_err(|e| {
                        diagnostic(
                            settings,
                            io::Error::new(
                                e.kind(),
                                format!("cannot start {:?}: {e}", settings.binary),
                            ),
                        )
                    })?,
                );
                on_status(ServicePhase::Starting);
                // The child now holds this same open file description. Keeping
                // the parent's copy until readiness also covers early startup.
            } else {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::WouldBlock {
                    return Err(diagnostic(settings, error));
                }
                if !waiting_reported {
                    on_status(ServicePhase::Waiting);
                    waiting_reported = true;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100).min(remaining));
    }
    let owned_startup = child.is_some();
    if let Some(mut owned) = child {
        // Terminate only a child started by this call, never a service merely
        // discovered at a socket or a PID read from an engine lock record.
        let _ = owned.kill();
        let _ = owned.wait();
    }
    if last_error.kind() == io::ErrorKind::Interrupted {
        return Err(diagnostic(settings, last_error));
    }
    let details = if owned_startup {
        format!(
            "; engine log {}: {}",
            log_path.display(),
            log_tail(&log_path)
        )
    } else {
        let mut owner = String::new();
        let _ = (&mut lock).take(8192).read_to_string(&mut owner);
        format!(
            "; engine weight lock {} is owned by another process: {}",
            settings.engine_lock.display(),
            owner.trim()
        )
    };
    Err(diagnostic(
        settings,
        io::Error::new(last_error.kind(), format!("{last_error}{details}")),
    ))
}

fn spawn_service(
    settings: &ServiceSettings,
    model: &Path,
    lock: &File,
    log_path: &Path,
) -> io::Result<Child> {
    let log = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(log_path)?;
    let mut command = Command::new(&settings.binary);
    command
        .arg("serve")
        .arg(model)
        .arg("--socket")
        .arg(&settings.socket)
        .arg("--idle-shutdown")
        .arg(settings.idle_shutdown_minutes.to_string())
        .arg("--state")
        .arg(&settings.engine_state_dir)
        .arg("--cache")
        .arg(&settings.engine_cache_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    if let Some(context) = settings.context {
        command.arg("--ctx").arg(context.to_string());
    }
    if let Some(mmproj) = &settings.mmproj {
        command.arg("--mmproj").arg(mmproj);
    }
    if let Some(budget) = settings.cache_budget {
        command.arg("--cache-budget").arg(budget.to_string());
    }
    let fd = lock.as_raw_fd();
    command.env("XENOLITH_ENGINE_LOCK_FD", fd.to_string());
    // SAFETY: only async-signal-safe system calls run between fork and exec.
    // FD_CLOEXEC is cleared in the child only, avoiding descriptor inheritance
    // races with other threads spawning unrelated processes in the parent.
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn()
}

fn log_tail(path: &Path) -> String {
    let result = (|| -> io::Result<String> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        file.seek(SeekFrom::Start(len.saturating_sub(LOG_TAIL_BYTES)))?;
        let mut bytes = Vec::new();
        file.take(LOG_TAIL_BYTES).read_to_end(&mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes).trim().to_owned())
    })();
    result.unwrap_or_else(|e| format!("cannot read startup diagnostics: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::BufRead;
    use std::io::BufReader;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    const INFO: &str = r#"{"ok":true,"protocol":1,"model":"fixture","context_window":16384,"max_output":2048,"max_frame":4096,"kvstore":true,"reasoning":{"efforts":["low"],"history":["discard"],"budget_tokens":true}}"#;

    struct Fixture {
        root: PathBuf,
        settings: ServiceSettings,
    }

    impl Fixture {
        fn new(mode: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "xe-service-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).unwrap();
            let binary = root.join("fake engine");
            let model = root.join("model $(must remain literal).gguf");
            fs::write(&model, mode).unwrap();
            fs::write(
                &binary,
                format!(
                    r##"#!/usr/bin/env python3
import fcntl, json, os, signal, socket, sys, time
args = sys.argv[1:]
assert args[0] == 'serve'
model = args[1]
mode = open(model).read()
path = args[args.index('--socket') + 1]
state = args[args.index('--state') + 1]
root = os.path.dirname(model)
open(root + '/args.json', 'w').write(json.dumps(args))
open(root + '/pid', 'w').write(str(os.getpid()))
with open(root + '/spawns', 'a') as record: record.write('spawn\n')
fd = int(os.environ['XENOLITH_ENGINE_LOCK_FD'])
fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
os.ftruncate(fd, 0)
os.write(fd, ('pid %d socket %s\n' % (os.getpid(), path)).encode())
# A separately opened descriptor must be excluded by the inherited lock.
other = os.open(root + '/.local/state/xenolith/engine.lock', os.O_RDWR)
try:
    fcntl.flock(other, fcntl.LOCK_EX | fcntl.LOCK_NB)
    raise AssertionError('inherited lock did not exclude another owner')
except BlockingIOError: pass
os.close(other)
print('fixture startup diagnostic', file=sys.stderr, flush=True)
if mode == 'exit': sys.exit(17)
if mode == 'stall': time.sleep(30)
time.sleep(0.12)
server = socket.socket(socket.AF_UNIX)
server.bind(path)
server.listen(8)
while True:
    peer, _ = server.accept()
    with peer:
        data = b''
        while not data.endswith(b'\n'):
            chunk = peer.recv(4096)
            if not chunk: break
            data += chunk
        if data:
            assert json.loads(data)['op'] == 'describe'
            peer.sendall(({INFO:?} + '\n').encode())
"##
                ),
            )
            .unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
            let env = HashMap::from([
                ("HOME", root.clone().into_os_string()),
                ("XENOLITH_BIN", binary.into_os_string()),
                ("XENOLITH_MODEL", model.into_os_string()),
            ]);
            let mut settings =
                ServiceSettings::from_lookup(None, |key| env.get(key).cloned()).unwrap();
            settings.startup_timeout = Duration::from_secs(3);
            settings.request_timeout = Duration::from_millis(200);
            Self { root, settings }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Ok(pid) = fs::read_to_string(self.root.join("pid"))
                && let Ok(pid) = pid.parse::<i32>()
            {
                // SAFETY: fixture records its own newly started child PID.
                unsafe {
                    libc::kill(pid, libc::SIGTERM);
                }
            }
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn projector_config_and_environment_precedence() {
        let lookup = |key: &str| (key == "HOME").then(|| OsString::from("/home/test"));
        let settings = ServiceSettings::from_lookup(None, lookup)
            .unwrap()
            .with_mmproj(Some(Path::new("/models/config.gguf")))
            .unwrap();
        assert_eq!(
            settings.mmproj.as_deref(),
            Some(Path::new("/models/config.gguf"))
        );
        assert!(
            ServiceSettings::from_lookup(None, lookup)
                .unwrap()
                .with_mmproj(Some(Path::new("relative.gguf")))
                .is_err()
        );
        let settings = ServiceSettings::from_lookup(None, |key| match key {
            "XENOLITH_MMPROJ" => Some("/models/env.gguf".into()),
            _ => lookup(key),
        })
        .unwrap()
        .with_mmproj(Some(Path::new("/models/config.gguf")))
        .unwrap();
        assert_eq!(
            settings.mmproj.as_deref(),
            Some(Path::new("/models/env.gguf"))
        );
    }

    #[test]
    fn paths_and_overrides_keep_engine_data_separate_from_native_bindings() {
        let env = HashMap::from([
            ("HOME", "/home/test"),
            ("XDG_RUNTIME_DIR", "/run/user/test"),
            ("XDG_STATE_HOME", "/state"),
            ("XDG_CACHE_HOME", "/cache"),
            ("XENOLITH_STATE_DIR", "/native-bindings"),
            ("XENOLITH_SOCKET", "/environment.sock"),
            ("XENOLITH_ENGINE_STATE_DIR", "/engine-data"),
            ("XENOLITH_ENGINE_CACHE_DIR", "/engine-cache"),
            ("XENOLITH_CONTEXT", "32768"),
            ("XENOLITH_CACHE_BUDGET", "4GiB"),
        ]);
        let lookup = |key: &str| env.get(key).map(OsString::from);
        let settings = ServiceSettings::from_lookup(Some("unix:///provider.sock"), lookup).unwrap();
        assert_eq!(settings.socket, Path::new("/provider.sock"));
        assert_eq!(settings.engine_state_dir, Path::new("/engine-data"));
        assert_eq!(settings.engine_cache_dir, Path::new("/engine-cache"));
        assert_eq!(
            settings.engine_lock,
            Path::new("/state/xenolith/engine.lock")
        );
        assert_eq!(settings.context, Some(32768));
        assert_eq!(settings.cache_budget, Some(4 * 1024 * 1024 * 1024));
        assert_eq!(
            ServiceSettings::from_lookup(None, lookup).unwrap().socket,
            Path::new("/environment.sock")
        );
        let env = HashMap::from([
            ("HOME", "/home/test"),
            ("XDG_STATE_HOME", "relative"),
            ("XDG_RUNTIME_DIR", "relative"),
        ]);
        let settings =
            ServiceSettings::from_lookup(None, |key| env.get(key).map(OsString::from)).unwrap();
        assert_eq!(
            settings.socket,
            Path::new("/home/test/.local/state/xenolith/wire.sock")
        );
        assert_eq!(
            settings.engine_cache_dir,
            Path::new("/home/test/.cache/xenolith/kv")
        );
    }

    #[test]
    fn invalid_environment_fails_explicitly() {
        for (key, value) in [
            ("XENOLITH_CONTEXT", "0"),
            ("XENOLITH_CONTEXT", "-1"),
            ("XENOLITH_CACHE_BUDGET", "18446744073709551615GiB"),
            ("XENOLITH_CACHE_BUDGET", "1.5GiB"),
            ("XENOLITH_IDLE_SHUTDOWN", "NaN"),
            ("XENOLITH_SOCKET", "relative.sock"),
            ("XENOLITH_ENGINE_STATE_DIR", "relative"),
        ] {
            let env = HashMap::from([("HOME", "/home/test"), (key, value)]);
            assert!(
                ServiceSettings::from_lookup(None, |key| env.get(key).map(OsString::from)).is_err(),
                "{key}={value}"
            );
        }
        for url in ["https://localhost", "unix://relative.sock"] {
            assert!(
                ServiceSettings::from_lookup(Some(url), |_| Some("/home/test".into())).is_err()
            );
        }
    }

    #[test]
    fn non_utf8_socket_is_rejected_before_core_can_store_a_lossy_endpoint() {
        use std::os::unix::ffi::OsStringExt;

        let lookup = |key: &str| match key {
            "HOME" => Some(OsString::from("/home/test")),
            "XENOLITH_SOCKET" => Some(OsString::from_vec(b"/tmp/wire-\xff.sock".to_vec())),
            _ => None,
        };
        let error = ServiceSettings::from_lookup(None, lookup).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(
            error
                .to_string()
                .contains("socket path must be valid UTF-8")
        );
        // A valid configured endpoint takes precedence over the environment.
        assert_eq!(
            ServiceSettings::from_lookup(Some("unix:///configured.sock"), lookup)
                .unwrap()
                .socket,
            Path::new("/configured.sock")
        );
    }

    #[test]
    fn existing_service_needs_no_model_and_reuses_describe_validation() {
        for reply in [
            INFO.to_owned(),
            INFO.replace("\"max_frame\":4096", "\"max_frame\":1"),
        ] {
            let mut fixture = Fixture::new("ready");
            private_directory(fixture.settings.socket.parent().unwrap()).unwrap();
            let listener = UnixListener::bind(&fixture.settings.socket).unwrap();
            let valid = reply == INFO;
            let server = std::thread::spawn(move || {
                let (mut peer, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(peer.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                assert_eq!(line, "{\"op\":\"describe\"}\n");
                writeln!(peer, "{reply}").unwrap();
            });
            fixture.settings.model = None;
            fixture.settings.binary = "/must/not/spawn".into();
            let result = ensure_service(&fixture.settings);
            assert_eq!(result.is_ok(), valid);
            assert!(!fixture.root.join("spawns").exists());
            server.join().unwrap();
        }
    }

    #[test]
    fn startup_is_shared_and_survives_short_discovery_connections() {
        let fixture = Fixture::new("ready");
        let settings = fixture.settings.clone();
        let concurrent = std::thread::spawn(move || ensure_service(&settings));
        assert_eq!(
            ensure_service(&fixture.settings).unwrap().context_window,
            16384
        );
        assert_eq!(concurrent.join().unwrap().unwrap().model, "fixture");
        assert_eq!(describe(&fixture.settings).unwrap().max_frame, 4096);
        assert_eq!(ensure_service(&fixture.settings).unwrap().model, "fixture");
        assert_eq!(
            fs::read_to_string(fixture.root.join("spawns")).unwrap(),
            "spawn\n"
        );
        let args: Vec<String> =
            serde_json::from_slice(&fs::read(fixture.root.join("args.json")).unwrap()).unwrap();
        assert_eq!(
            args[1],
            fixture.settings.model.as_ref().unwrap().to_str().unwrap()
        );
        assert!(args.windows(2).any(|a| a == ["--idle-shutdown", "30"]));
    }

    #[test]
    fn startup_phases_follow_real_child_and_validated_readiness() {
        let fixture = Fixture::new("ready");
        let mut phases = Vec::new();
        ensure_service_with_status(&fixture.settings, |phase| phases.push(phase)).unwrap();
        assert_eq!(
            phases,
            [
                ServicePhase::Connecting,
                ServicePhase::Starting,
                ServicePhase::Ready
            ]
        );
        phases.clear();
        ensure_service_with_status(&fixture.settings, |phase| phases.push(phase)).unwrap();
        assert_eq!(phases, [ServicePhase::Connecting, ServicePhase::Ready]);
        assert_eq!(
            fs::read_to_string(fixture.root.join("spawns")).unwrap(),
            "spawn\n"
        );
    }

    #[test]
    fn failed_startup_reports_starting_without_fabricating_ready_and_reaps_child() {
        let mut fixture = Fixture::new("stall");
        fixture.settings.startup_timeout = Duration::from_millis(300);
        let mut phases = Vec::new();
        let error =
            ensure_service_with_status(&fixture.settings, |phase| phases.push(phase)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(phases, [ServicePhase::Connecting, ServicePhase::Starting]);
        let pid: i32 = fs::read_to_string(fixture.root.join("pid"))
            .unwrap()
            .parse()
            .unwrap();
        // SAFETY: signal zero checks only this known fixture child's existence.
        assert_ne!(unsafe { libc::kill(pid, 0) }, 0);
    }

    #[test]
    fn cancellation_reaps_a_pending_child_without_waiting_for_readiness_timeout() {
        let mut fixture = Fixture::new("stall");
        fixture.settings.startup_timeout = Duration::from_secs(120);
        let settings = fixture.settings.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = cancelled.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            ensure_service_cancellable(&settings, signal, |phase| {
                tx.send(phase).unwrap();
            })
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            ServicePhase::Connecting
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            ServicePhase::Starting
        );
        let pid_path = fixture.root.join("pid");
        let deadline = Instant::now() + Duration::from_secs(2);
        let pid: i32 = loop {
            if let Ok(text) = fs::read_to_string(&pid_path)
                && let Ok(pid) = text.parse()
            {
                break pid;
            }
            assert!(
                Instant::now() < deadline,
                "fixture must start before cancellation"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        let started = Instant::now();
        cancelled.store(true, Ordering::Release);
        assert_eq!(
            worker.join().unwrap().unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(
            rx.try_recv().is_err(),
            "cancelled startup must not emit ready"
        );
        // SAFETY: signal zero checks only this known fixture child's existence.
        assert_ne!(unsafe { libc::kill(pid, 0) }, 0);
    }

    #[test]
    fn cancellation_interrupts_stalled_describe_without_cancelling_external_service() {
        let mut fixture = Fixture::new("ready");
        fixture.settings.request_timeout = Duration::from_secs(120);
        private_directory(fixture.settings.socket.parent().unwrap()).unwrap();
        let listener = UnixListener::bind(&fixture.settings.socket).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (peer, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(peer);
            let mut frame = String::new();
            reader.read_line(&mut frame).unwrap();
            assert_eq!(frame, "{\"op\":\"describe\"}\n");
            tx.send(()).unwrap();
            frame.clear();
            assert_eq!(
                reader.read_line(&mut frame).unwrap(),
                0,
                "discovery cancellation closes this socket without a wire cancel request"
            );
            let (mut peer, _) = listener.accept().unwrap();
            BufReader::new(peer.try_clone().unwrap())
                .read_line(&mut frame)
                .unwrap();
            assert_eq!(frame, "{\"op\":\"describe\"}\n");
            writeln!(peer, "{INFO}").unwrap();
        });
        let settings = fixture.settings.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = cancelled.clone();
        let worker =
            std::thread::spawn(move || ensure_service_cancellable(&settings, signal, |_| {}));
        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let started = Instant::now();
        cancelled.store(true, Ordering::Release);
        assert_eq!(
            worker.join().unwrap().unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(describe(&fixture.settings).unwrap().model, "fixture");
        assert!(!fixture.root.join("spawns").exists());
        server.join().unwrap();
    }

    #[test]
    fn cancellation_after_ready_preserves_the_detached_service() {
        let fixture = Fixture::new("ready");
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = cancelled.clone();
        ensure_service_cancellable(&fixture.settings, cancelled, |phase| {
            if phase == ServicePhase::Ready {
                signal.store(true, Ordering::Release);
            }
        })
        .unwrap();
        assert_eq!(describe(&fixture.settings).unwrap().model, "fixture");
        assert_eq!(
            fs::read_to_string(fixture.root.join("spawns")).unwrap(),
            "spawn\n"
        );
    }

    #[test]
    fn global_lock_blocks_another_socket_without_stopping_the_owner() {
        let fixture = Fixture::new("ready");
        ensure_service(&fixture.settings).unwrap();
        let mut another = fixture.settings.clone();
        another.socket = fixture.root.join("another.sock");
        another.startup_timeout = Duration::from_millis(200);
        let mut phases = Vec::new();
        let message = ensure_service_with_status(&another, |phase| phases.push(phase))
            .unwrap_err()
            .to_string();
        assert_eq!(phases, [ServicePhase::Connecting, ServicePhase::Waiting]);
        assert!(message.contains("engine weight lock"), "{message}");
        assert!(message.contains("owned by another process"), "{message}");
        assert_eq!(
            fs::read_to_string(fixture.root.join("spawns")).unwrap(),
            "spawn\n"
        );
        assert!(describe(&fixture.settings).is_ok());
    }

    #[test]
    fn disabled_and_unconfigured_startup_report_actionable_errors() {
        let mut fixture = Fixture::new("ready");
        fixture.settings.spawn = false;
        assert!(
            ensure_service(&fixture.settings)
                .unwrap_err()
                .to_string()
                .contains("XENOLITH_NO_SPAWN")
        );
        fixture.settings.spawn = true;
        fixture.settings.model = None;
        assert!(
            ensure_service(&fixture.settings)
                .unwrap_err()
                .to_string()
                .contains("XENOLITH_MODEL")
        );
        assert!(!fixture.root.join("spawns").exists());
    }

    #[test]
    fn failed_exec_and_child_exit_include_startup_diagnostics() {
        let mut fixture = Fixture::new("exit");
        let message = ensure_service(&fixture.settings).unwrap_err().to_string();
        assert!(message.contains("exit status: 17"), "{message}");
        assert!(message.contains("fixture startup diagnostic"), "{message}");
        fixture.settings.binary = "/missing/xenolith".into();
        let message = ensure_service(&fixture.settings).unwrap_err().to_string();
        assert!(message.contains("cannot start"), "{message}");
    }

    #[test]
    fn readiness_timeout_reaps_only_the_child_started_by_this_call() {
        let mut fixture = Fixture::new("stall");
        fixture.settings.startup_timeout = Duration::from_millis(300);
        let error = ensure_service(&fixture.settings).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("fixture startup diagnostic"));
        let pid: i32 = fs::read_to_string(fixture.root.join("pid"))
            .unwrap()
            .parse()
            .unwrap();
        // SAFETY: signal zero checks only this known fixture child's existence.
        assert_ne!(unsafe { libc::kill(pid, 0) }, 0);
    }
}
