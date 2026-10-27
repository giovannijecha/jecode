use crate::cancel::Cancellation;
use std::fs::{self, File};
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, TryLockError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SOURCE: &str = include_str!("unix.c");
const SESSION: &str = include_str!("unix_session.h");
const START_LIMIT: Duration = Duration::from_secs(5);
const COMPILE_LIMIT: Duration = Duration::from_secs(15);
static BUILD: Mutex<()> = Mutex::new(());

/// Wraps the target in the owned native PTY bridge. The caller launches this
/// command with `process::start_logged`, preserving the normal Unix guardian.
pub(super) fn command(
    target: &Command,
    endpoint: &str,
    columns: u16,
    rows: u16,
    cancellation: &Cancellation,
) -> Result<Command, String> {
    if columns == 0 || rows == 0 {
        return Err("PTY dimensions must be positive".into());
    }
    let helper = helper(cancellation)?;
    let mut command = Command::new(helper);
    command
        .arg(endpoint)
        .arg(columns.to_string())
        .arg(rows.to_string())
        .arg(target.get_program())
        .args(target.get_args())
        .stdin(Stdio::null());
    if let Some(directory) = target.get_current_dir() {
        command.current_dir(directory);
    }
    for (key, value) in target.get_envs() {
        if let Some(value) = value {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
    }
    Ok(command)
}

/// Connects to the bridge's private Unix socket after it has begun listening.
/// The returned File is nonblocking so a full input queue cannot stall UI work.
pub(super) fn connect(endpoint: &str, cancellation: &Cancellation) -> Result<File, String> {
    let started = Instant::now();
    loop {
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        match UnixStream::connect(endpoint) {
            Ok(stream) => {
                stream
                    .set_nonblocking(true)
                    .map_err(|error| format!("Could not configure Unix PTY input: {error}"))?;
                let descriptor: OwnedFd = stream.into();
                return Ok(File::from(descriptor));
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) && started.elapsed() < START_LIMIT =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(format!("Could not connect to Unix PTY: {error}")),
        }
    }
}

pub(super) fn connect_control(endpoint: &str, cancellation: &Cancellation) -> Result<File, String> {
    connect(endpoint, cancellation)
}

fn helper(cancellation: &Cancellation) -> Result<PathBuf, String> {
    if cancellation.requested() {
        return Err("Operation cancelled".into());
    }
    let _lock = loop {
        match BUILD.try_lock() {
            Ok(lock) => break lock,
            Err(TryLockError::WouldBlock) if !cancellation.requested() => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(TryLockError::Poisoned(_)) => {
                return Err("Unix PTY compiler lock failed".into());
            }
            Err(TryLockError::WouldBlock) => return Err("Operation cancelled".into()),
        }
    };
    let root = cache_root()?;
    fs::create_dir_all(&root)
        .map_err(|error| format!("Could not create Unix PTY cache: {error}"))?;
    let final_path = root.join(format!(
        "bridge-{}-{}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        fingerprint(&format!("{SOURCE}{SESSION}"))
    ));
    if fs::metadata(&final_path).is_ok_and(|entry| entry.is_file() && entry.len() > 0) {
        return Ok(final_path);
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("Could not read system time: {error}"))?
        .as_nanos();
    let temporary = root.join(format!("build-{}-{nonce}", std::process::id()));
    let source = temporary.join("unix.c");
    let header = temporary.join("unix_session.h");
    let executable = temporary.join("bridge");
    let result = (|| {
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        fs::create_dir(&temporary)
            .map_err(|error| format!("Could not stage Unix PTY helper: {error}"))?;
        fs::write(&source, SOURCE)
            .map_err(|error| format!("Could not stage Unix PTY source: {error}"))?;
        fs::write(&header, SESSION)
            .map_err(|error| format!("Could not stage Unix PTY source: {error}"))?;
        let mut compiler = Command::new("cc");
        compiler
            .args(["-std=c11", "-O2", "-Wall", "-Wextra", "-o"])
            .arg(&executable)
            .arg(&source)
            .stdin(Stdio::null());
        crate::auth::isolate_environment(&mut compiler);
        let output = crate::process::run_observed(
            &mut compiler,
            None,
            COMPILE_LIMIT,
            8192,
            cancellation,
            None,
            &mut |_| Ok(()),
        )
        .map_err(|error| format!("Could not run system C compiler: {error}"))?;
        if output.cancelled {
            return Err("Operation cancelled".into());
        }
        if output.timed_out {
            return Err("Unix PTY helper compilation timed out".into());
        }
        if output.exit_code != Some(0) {
            return Err(format!(
                "Could not compile Unix PTY helper with cc: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        if !fs::metadata(&executable).is_ok_and(|entry| entry.is_file() && entry.len() > 0) {
            return Err("Unix PTY compiler produced no executable".into());
        }
        match fs::rename(&executable, &final_path) {
            Ok(()) => Ok(final_path),
            Err(_) if fs::metadata(&final_path).is_ok_and(|entry| entry.is_file()) => {
                Ok(final_path)
            }
            Err(error) => Err(format!("Could not cache Unix PTY helper: {error}")),
        }
    })();
    let _ = fs::remove_dir_all(&temporary);
    result
}

fn cache_root() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/measurements/pty-cache"))
    }
    #[cfg(not(test))]
    {
        Ok(crate::config::Store::discover()?
            .path()
            .parent()
            .ok_or("Could not locate the Jecode configuration directory")?
            .join("cache/pty"))
    }
}

fn fingerprint(source: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in source.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
#[path = "unix_tests.rs"]
mod tests;
