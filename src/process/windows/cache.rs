//! Compiles the owned .NET Framework supervisor once per source version.
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, TryLockError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::cancel::Cancellation;

const SOURCE: &str = include_str!("../windows.cs");
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const COMPILE_LIMIT: Duration = Duration::from_secs(15);
static SUPERVISOR: Mutex<State> = Mutex::new(State::Uninitialized);

enum State {
    Uninitialized,
    Ready(PathBuf),
    Unavailable,
}

#[derive(Debug, PartialEq, Eq)]
enum CompileError {
    Cancelled,
    Failed,
}

pub(super) fn supervisor(cancellation: &Cancellation) -> Option<PathBuf> {
    if cancellation.requested() {
        return None;
    }
    let mut state = loop {
        match SUPERVISOR.try_lock() {
            Ok(state) => break state,
            Err(TryLockError::WouldBlock) if !cancellation.requested() => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    };
    if cancellation.requested() {
        return None;
    }
    match &*state {
        State::Ready(path) => return Some(path.clone()),
        State::Unavailable => return None,
        State::Uninitialized => {}
    }
    match compile(cancellation) {
        Ok(path) => {
            *state = State::Ready(path.clone());
            Some(path)
        }
        Err(CompileError::Cancelled) => None,
        Err(CompileError::Failed) => {
            *state = State::Unavailable;
            None
        }
    }
}

fn root() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/measurements")
            .join(format!("supervisor-cache-{}-{nonce}", std::process::id())))
    }
    #[cfg(not(test))]
    {
        let home = std::env::var_os("JECODE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".jecode"))
            })
            .ok_or("Could not locate the user configuration directory")?;
        if !home.is_absolute() {
            return Err("Supervisor cache directory must be absolute".into());
        }
        Ok(home.join("cache/process"))
    }
}

fn fingerprint() -> String {
    // Two independent FNV-1a streams make source changes invalidate the cache.
    // This is a version key, not an authenticity check for the user-owned cache.
    let mut a = 0xcbf29ce484222325u64;
    let mut b = 0x84222325cbf29ce4u64;
    for byte in SOURCE.bytes() {
        a = (a ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        b = (b ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    format!("{a:016x}{b:016x}")
}

fn compile(cancellation: &Cancellation) -> Result<PathBuf, CompileError> {
    if cancellation.requested() {
        return Err(CompileError::Cancelled);
    }
    compile_at(root().map_err(|_| CompileError::Failed)?, cancellation)
}

fn compile_at(root: PathBuf, cancellation: &Cancellation) -> Result<PathBuf, CompileError> {
    if cancellation.requested() {
        return Err(CompileError::Cancelled);
    }
    fs::create_dir_all(&root).map_err(|_| CompileError::Failed)?;
    let final_path = root.join(format!("supervisor-v1-{}.exe", fingerprint()));
    if fs::metadata(&final_path).is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0) {
        return Ok(final_path);
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CompileError::Failed)?
        .as_nanos();
    let temporary = root.join(format!("build-{}-{nonce}", std::process::id()));
    let source = temporary.with_extension("cs");
    let executable = temporary.with_extension("exe");
    let result = (|| {
        if cancellation.requested() {
            return Err(CompileError::Cancelled);
        }
        fs::write(&source, SOURCE).map_err(|_| CompileError::Failed)?;
        let quote = |path: &PathBuf| path.to_string_lossy().replace('\'', "''");
        let script = format!(
            "$ErrorActionPreference = 'Stop'\nAdd-Type -LiteralPath '{}' -OutputAssembly '{}' -OutputType ConsoleApplication",
            quote(&source),
            quote(&executable)
        );
        let mut compiler = Command::new("powershell.exe");
        compiler
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &script,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .env_remove("OPENROUTER_API_KEY");
        run_compiler(&mut compiler, cancellation, COMPILE_LIMIT)?;
        if cancellation.requested() {
            return Err(CompileError::Cancelled);
        }
        if !fs::metadata(&executable).is_ok_and(|metadata| metadata.len() > 0) {
            return Err(CompileError::Failed);
        }
        match fs::rename(&executable, &final_path) {
            Ok(()) => Ok(final_path),
            Err(_) if fs::metadata(&final_path).is_ok_and(|metadata| metadata.len() > 0) => {
                Ok(final_path)
            }
            Err(_) => Err(CompileError::Failed),
        }
    })();
    let _ = fs::remove_file(&source);
    let _ = fs::remove_file(&executable);
    result
}

fn run_compiler(
    command: &mut Command,
    cancellation: &Cancellation,
    limit: Duration,
) -> Result<(), CompileError> {
    if cancellation.requested() {
        return Err(CompileError::Cancelled);
    }
    let mut child = command.spawn().map_err(|_| CompileError::Failed)?;
    let started = Instant::now();
    loop {
        if cancellation.requested() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(CompileError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(CompileError::Failed)
                };
            }
            Ok(None) if started.elapsed() < limit => thread::sleep(Duration::from_millis(20)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CompileError::Failed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;

    const FIXTURE: &str = "process::windows::cache::tests::compiler_fixture";

    #[test]
    fn compiler_fixture() {
        let Some(marker) = std::env::var_os("JECODE_COMPILER_FIXTURE") else {
            return;
        };
        let marker = PathBuf::from(marker);
        fs::write(&marker, b"started").unwrap();
        thread::sleep(Duration::from_secs(5));
        fs::write(marker.with_extension("completed"), b"completed").unwrap();
    }

    #[test]
    fn cancelled_before_cold_compile_starts_no_compiler_or_cache_write() {
        let directory = Directory::new();
        let cache = directory.path().join("cache");
        let cancellation = Cancellation::default();
        cancellation.cancel();
        assert_eq!(
            compile_at(cache.clone(), &cancellation),
            Err(CompileError::Cancelled)
        );
        assert!(!cache.exists());
    }

    #[test]
    fn cancellation_reaps_a_running_compiler() {
        let directory = Directory::new();
        let marker = directory.path().join("compiler-started");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", FIXTURE])
            .env("JECODE_COMPILER_FIXTURE", &marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let cancellation = Cancellation::default();
        let signal = cancellation.clone();
        let worker =
            thread::spawn(move || run_compiler(&mut command, &signal, Duration::from_secs(3)));
        let started = Instant::now();
        while !marker.exists() && started.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(marker.exists(), "compiler fixture did not start");
        cancellation.cancel();
        assert_eq!(worker.join().unwrap(), Err(CompileError::Cancelled));
        assert!(!marker.with_extension("completed").exists());
    }

    #[test]
    fn deadline_reaps_a_stalled_compiler() {
        let directory = Directory::new();
        let marker = directory.path().join("compiler-started");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", FIXTURE])
            .env("JECODE_COMPILER_FIXTURE", &marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let started = Instant::now();
        assert_eq!(
            run_compiler(
                &mut command,
                &Cancellation::default(),
                Duration::from_millis(100),
            ),
            Err(CompileError::Failed)
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!marker.with_extension("completed").exists());
    }

    #[test]
    fn cancellation_does_not_wait_for_another_cold_start() {
        let state = SUPERVISOR.lock().unwrap();
        let cancellation = Cancellation::default();
        let signal = cancellation.clone();
        let (send, receive) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let _ = send.send(supervisor(&signal));
        });
        thread::sleep(Duration::from_millis(30));
        cancellation.cancel();
        let result = receive.recv_timeout(Duration::from_millis(500));
        drop(state);
        worker.join().unwrap();
        assert_eq!(result.unwrap(), None);
    }
}
