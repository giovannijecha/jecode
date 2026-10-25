use std::ffi::OsString;
use std::process::Command;
use std::time::Duration;

use crate::cancel::Cancellation;
use crate::process::{self, Output};

pub(crate) fn run(
    command: &mut Command,
    input: Option<Vec<u8>>,
    timeout: impl Into<Option<Duration>>,
    output_limit: usize,
    cancellation: &Cancellation,
    idle: Option<Duration>,
    observer: &mut impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<Output, String> {
    let timeout = timeout.into();
    if known_system_curl(command) {
        process::run_leaf(
            command,
            input,
            timeout,
            output_limit,
            cancellation,
            idle,
            observer,
        )
    } else {
        process::run_observed(
            command,
            input,
            timeout,
            output_limit,
            cancellation,
            idle,
            observer,
        )
    }
}

#[cfg(windows)]
fn known_system_curl(command: &Command) -> bool {
    if std::env::var_os("JECODE_CURL").is_some_and(|value| !value.is_empty()) {
        return false;
    }
    let Some(root) = std::env::var_os("SystemRoot") else {
        return false;
    };
    let system_curl = std::path::PathBuf::from(root).join("System32/curl.exe");
    system_curl.is_file() && command.get_program() == system_curl.as_os_str()
}

#[cfg(unix)]
fn known_system_curl(_command: &Command) -> bool {
    // Both runners retain the same process-group boundary on Unix.
    true
}

/// Honor an explicit curl override, then prefer the Windows system executable.
pub(crate) fn executable() -> OsString {
    if let Some(path) = std::env::var_os("JECODE_CURL").filter(|value| !value.is_empty()) {
        return path;
    }
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        let path = std::path::PathBuf::from(root).join("System32/curl.exe");
        if path.is_file() {
            return path.into_os_string();
        }
    }
    "curl".into()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::test_support::Directory;

    #[test]
    fn custom_executable_cancellation_stops_its_child() {
        let directory = Directory::new();
        let marker = directory.path().join("child-survived");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "process::tests::exited_root_fixture"])
            .env("JECODE_PROCESS_FIXTURE", "root")
            .env("JECODE_PROCESS_MARKER", &marker);
        let cancellation = Cancellation::default();
        let signal = cancellation.clone();
        let mut observed = Vec::new();
        run(
            &mut command,
            None,
            Some(Duration::from_secs(5)),
            4096,
            &cancellation,
            None,
            &mut |bytes| {
                observed.extend_from_slice(bytes);
                if String::from_utf8_lossy(&observed).contains("root finished") {
                    signal.cancel();
                }
                Ok(())
            },
        )
        .expect("custom executable must launch and finish under supervision");
        assert!(
            cancellation.requested(),
            "wrapper did not reach cancellation"
        );
        std::thread::sleep(Duration::from_millis(2300));
        assert!(
            !marker.exists(),
            "custom executable child survived cancellation"
        );
    }
}
