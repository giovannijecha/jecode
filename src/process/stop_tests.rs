use super::*;
use crate::sandbox::MARKER;
use crate::test_support::Directory;

fn shell(directory: &Directory, text: &str) -> Command {
    let mut command = crate::shell::command(text).unwrap();
    command.current_dir(directory.path());
    command
}

fn captured(command: &mut Command) -> Started {
    start(
        command,
        None,
        &Cancellation::default(),
        CaptureOptions {
            limit: 4096,
            tail: false,
            spools: [None, None],
            observed: false,
            idle: None,
            leaf: false,
        },
    )
    .unwrap()
}

#[test]
fn killed_supervisor_still_reports_the_timeout_and_its_output() {
    let directory = Directory::new();
    let mut started = captured(&mut shell(&directory, "printf before; sleep 20"));
    // Without its stop channel the boundary falls back to killing the
    // supervisor, which can no longer send a completion report.
    started.control = None;
    let launched = Instant::now();
    let output = started
        .supervise(
            Some(Duration::from_millis(1500)),
            &Cancellation::default(),
            &mut |_| Ok(()),
        )
        .unwrap_or_else(|error| panic!("{}", error.message));
    assert!(output.timed_out && !output.cancelled);
    assert_eq!(output.exit_code, None);
    assert_eq!(output.stdout, b"before");
    assert!(launched.elapsed() < Duration::from_secs(10));
}

#[test]
fn killed_supervisor_still_reports_cancellation() {
    let directory = Directory::new();
    let mut started = captured(&mut shell(&directory, "printf before; sleep 20"));
    started.control = None;
    let cancellation = Cancellation::default();
    let signal = cancellation.clone();
    let timer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        signal.cancel();
    });
    let output = started
        .supervise(None, &cancellation, &mut |_| Ok(()))
        .unwrap_or_else(|error| panic!("{}", error.message));
    timer.join().unwrap();
    assert!(output.cancelled && !output.timed_out);
    assert_eq!(output.stdout, b"before");
}

#[test]
fn empty_sandbox_marker_is_treated_as_absent() {
    let mut command = Command::new("cmd.exe");
    assert!(sandbox_policy(&command).is_none());
    command.env(MARKER, "");
    assert!(sandbox_policy(&command).is_none());
    command.env(MARKER.to_ascii_lowercase(), "{}");
    assert_eq!(sandbox_policy(&command), Some(std::ffi::OsStr::new("{}")));
}

#[test]
fn verbatim_directories_become_ordinary_paths() {
    use std::path::Path;
    assert_eq!(
        ordinary_path(Path::new(r"\\?\C:\Work")),
        Path::new(r"C:\Work")
    );
    assert_eq!(
        ordinary_path(Path::new(r"\\?\UNC\server\share\dir")),
        Path::new(r"\\server\share\dir")
    );
    assert_eq!(ordinary_path(Path::new(r"C:\Work")), Path::new(r"C:\Work"));
}

#[test]
fn malformed_sandbox_policy_never_starts_the_command() {
    let directory = Directory::new();
    for (policy, expected) in [
        (
            r#"{"read":["C:\\x"],"network":true"#,
            "Invalid Windows sandbox policy",
        ),
        // Escapes are decoded before the paths are validated.
        (
            r#"{"deny":[],"network":true,"read":["\u0041:relative"],"write":[]}"#,
            "Sandbox paths must be canonical and absolute",
        ),
    ] {
        let mut command = shell(&directory, "printf ran > marker");
        command.env(MARKER, policy);
        let error = windows::spawn(&command, None, &Cancellation::default())
            .err()
            .unwrap();
        assert!(!error.started);
        assert!(error.message.contains(expected), "{}", error.message);
    }
    assert!(!directory.path().join("marker").exists());
}
