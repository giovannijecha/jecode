use super::*;
use crate::{output, redact::Redactor, test_support::Directory};

#[cfg(windows)]
const CHILD_FIXTURE: &str = "process::tests::exited_root_fixture";

#[cfg(windows)]
#[test]
fn exited_root_fixture() {
    use std::io::Write;
    let Some(mode) = std::env::var_os("JECODE_PROCESS_FIXTURE") else {
        return;
    };
    if mode == "root" {
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CHILD_FIXTURE])
            .env("JECODE_PROCESS_FIXTURE", "child")
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        if let Some(marker) = std::env::var_os("JECODE_PROCESS_ROOT_MARKER") {
            std::fs::write(marker, b"started").unwrap();
        }
        std::io::stdout().write_all(b"root finished\n").unwrap();
        std::io::stdout().flush().unwrap();
        std::process::exit(0);
    }
    std::thread::sleep(Duration::from_secs(2));
    std::io::stdout().write_all(b"child finished\n").unwrap();
    std::fs::write(
        std::env::var_os("JECODE_PROCESS_MARKER").unwrap(),
        b"survived",
    )
    .unwrap();
}

#[cfg(windows)]
#[test]
fn owner_exit_fixture() {
    if std::env::var_os("JECODE_PROCESS_OWNER_FIXTURE").is_none() {
        return;
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", CHILD_FIXTURE])
        .env("JECODE_PROCESS_FIXTURE", "root");
    let _ = run_observed(
        &mut command,
        None,
        None,
        4096,
        &Cancellation::default(),
        None,
        &mut |bytes| {
            if String::from_utf8_lossy(bytes).contains("root finished") {
                std::process::exit(0);
            }
            Ok(())
        },
    );
    panic!("owner fixture did not exit after target started");
}

#[cfg(windows)]
#[test]
fn owner_disappearance_stops_the_job() {
    let directory = Directory::new();
    let survivor = directory.path().join("survivor");
    let root_marker = directory.path().join("root-started");
    let mut owner = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "process::tests::owner_exit_fixture"])
        .env("JECODE_PROCESS_OWNER_FIXTURE", "1")
        .env("JECODE_PROCESS_MARKER", &survivor)
        .env("JECODE_PROCESS_ROOT_MARKER", &root_marker)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !root_marker.exists() && started.elapsed() < Duration::from_secs(8) {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !root_marker.exists() {
        let _ = owner.kill();
        let _ = owner.wait();
        panic!("target did not start");
    }
    while owner.try_wait().unwrap().is_none() && started.elapsed() < Duration::from_secs(8) {
        std::thread::sleep(Duration::from_millis(10));
    }
    if owner.try_wait().unwrap().is_none() {
        let _ = owner.kill();
        let _ = owner.wait();
        panic!("owner did not exit");
    }
    std::thread::sleep(Duration::from_millis(2200));
    assert!(!survivor.exists(), "job survived its owner");
}

#[cfg(windows)]
#[test]
fn supervisor_exit_without_completion_is_unknown() {
    let directory = Directory::new();
    let spawned = windows::spawn(
        &command(&directory, "sleep 20"),
        None,
        &Cancellation::default(),
    )
    .unwrap();
    let windows::Spawned {
        mut child,
        completion,
        ..
    } = spawned;
    child.kill().unwrap();
    child.wait().unwrap();
    let error = windows::completion(completion).unwrap_err();
    assert!(error.contains("outcome is unknown"), "{error}");
}

#[cfg(windows)]
fn descendant_command(directory: &Directory) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", CHILD_FIXTURE])
        .env("JECODE_PROCESS_FIXTURE", "root")
        .env("JECODE_PROCESS_MARKER", directory.path().join("survivor"));
    command
}

#[cfg(windows)]
#[test]
fn clean_exit_waits_for_descendant_and_keeps_its_output() {
    let directory = Directory::new();
    let started = Instant::now();
    let output = run_observed(
        &mut descendant_command(&directory),
        None,
        None,
        4096,
        &Cancellation::default(),
        None,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(
        output.exit_code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.cancelled && !output.timed_out);
    assert!(started.elapsed() >= Duration::from_secs(2));
    assert!(String::from_utf8_lossy(&output.stdout).contains("child finished"));
    assert!(directory.path().join("survivor").exists());
}

#[cfg(windows)]
#[test]
fn pathless_executable_uses_the_windows_search_path() {
    let mut command = Command::new("curl.exe");
    command.arg("--version");
    let output = run_observed(
        &mut command,
        None,
        None,
        4096,
        &Cancellation::default(),
        None,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(
        output.exit_code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("curl "));
}

#[cfg(windows)]
#[test]
fn cancellation_stops_descendant_after_root_exits() {
    let directory = Directory::new();
    let cancellation = Cancellation::default();
    let signal = cancellation.clone();
    let started = Instant::now();
    let output = run_observed(
        &mut descendant_command(&directory),
        None,
        None,
        4096,
        &cancellation,
        None,
        &mut |bytes| {
            if String::from_utf8_lossy(bytes).contains("root finished") {
                std::thread::sleep(Duration::from_millis(250));
                signal.cancel();
            }
            Ok(())
        },
    )
    .unwrap();
    assert!(output.cancelled);
    assert!(String::from_utf8_lossy(&output.stdout).contains("root finished"));
    assert!(started.elapsed() < Duration::from_secs(2));
    std::thread::sleep(Duration::from_millis(2200));
    assert!(!directory.path().join("survivor").exists());
}

#[cfg(windows)]
#[test]
fn timeout_stops_descendant_after_root_exits() {
    let directory = Directory::new();
    let started = Instant::now();
    let output = run_observed(
        &mut descendant_command(&directory),
        None,
        Duration::from_millis(700),
        4096,
        &Cancellation::default(),
        None,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert!(output.timed_out);
    assert!(String::from_utf8_lossy(&output.stdout).contains("root finished"));
    assert!(!output.cancelled);
    assert!(started.elapsed() < Duration::from_secs(3));
    std::thread::sleep(Duration::from_millis(2200));
    assert!(!directory.path().join("survivor").exists());
}

fn command(directory: &Directory, text: &str) -> Command {
    let mut command = Command::new(crate::tools::find_bash().unwrap());
    command
        .args(["--noprofile", "--norc", "-c", text])
        .current_dir(directory.path())
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("BASH_ENV")
        .env_remove("ENV");
    command
}

#[cfg(unix)]
const BACKGROUND_CHILD: &str =
    "(sleep 2; printf child-finished; printf survived > survivor) & printf 'root-finished\\n'";

#[cfg(unix)]
#[test]
fn cleanup_accepts_an_already_finished_process_group() {
    use std::os::unix::process::CommandExt;
    let mut child = Command::new("bash")
        .args(["--noprofile", "--norc", "-c", "true"])
        .process_group(0)
        .spawn()
        .unwrap();
    child.wait().unwrap();
    terminate(&mut child).unwrap();
}

#[cfg(unix)]
#[test]
fn clean_exit_waits_for_descendant_after_root_exits() {
    let directory = Directory::new();
    let started = Instant::now();
    let output = run_observed(
        &mut command(&directory, BACKGROUND_CHILD),
        None,
        None,
        4096,
        &Cancellation::default(),
        None,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert!(started.elapsed() >= Duration::from_secs(2));
    assert!(String::from_utf8_lossy(&output.stdout).contains("child-finished"));
    assert!(directory.path().join("survivor").exists());
}

#[cfg(unix)]
#[test]
fn cancellation_stops_descendant_after_root_exits() {
    let directory = Directory::new();
    let cancellation = Cancellation::default();
    let signal = cancellation.clone();
    let output = run_observed(
        &mut command(&directory, BACKGROUND_CHILD),
        None,
        None,
        4096,
        &cancellation,
        None,
        &mut |bytes| {
            if String::from_utf8_lossy(bytes).contains("root-finished") {
                std::thread::sleep(Duration::from_millis(250));
                signal.cancel();
            }
            Ok(())
        },
    )
    .unwrap();
    assert!(output.cancelled);
    assert!(String::from_utf8_lossy(&output.stdout).contains("root-finished"));
    std::thread::sleep(Duration::from_millis(2200));
    assert!(!directory.path().join("survivor").exists());
}

#[cfg(unix)]
#[test]
fn timeout_stops_descendant_after_root_exits() {
    let directory = Directory::new();
    let output = run_observed(
        &mut command(&directory, BACKGROUND_CHILD),
        None,
        Duration::from_millis(700),
        4096,
        &Cancellation::default(),
        None,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert!(output.timed_out);
    assert!(String::from_utf8_lossy(&output.stdout).contains("root-finished"));
    std::thread::sleep(Duration::from_millis(2200));
    assert!(!directory.path().join("survivor").exists());
}

#[test]
fn cancellation_before_spawn_never_runs_the_command() {
    let directory = Directory::new();
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let result = run_observed(
        &mut command(&directory, "printf ran > marker"),
        None,
        None,
        4096,
        &cancellation,
        None,
        &mut |_| Ok(()),
    );
    assert_eq!(result.err().as_deref(), Some("Operation cancelled"));
    assert!(!directory.path().join("marker").exists());
}

#[test]
fn response_activity_extends_the_idle_deadline_without_limiting_total_duration() {
    let directory = Directory::new();
    let started = Instant::now();
    let mut command = command(
        &directory,
        "for value in 1 2 3; do printf 'heartbeat\\n'; sleep 0.45; done",
    );
    let output = run_observed(
        &mut command,
        None,
        None,
        4096,
        &Cancellation::default(),
        Some(Duration::from_secs(1)),
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(
        output.exit_code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.timed_out);
    assert!(started.elapsed() > Duration::from_secs(1));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "heartbeat\n".repeat(3)
    );
}

#[test]
fn output_failure_after_start_is_distinguished_from_a_command_that_never_started() {
    let directory = Directory::new();
    let store = output::Store::new(directory.path().join("logs"), Redactor::empty());
    let probe = directory.path().join("read-handle");
    std::fs::write(&probe, "fixture").unwrap();
    let logs = store.create().unwrap();
    let error = run_logged(
        &mut command(&directory, "printf 'executed' > marker; printf 'output'"),
        None,
        4096,
        &Cancellation::default(),
        [output::Spool::read_only_fixture(&probe), logs.stderr],
    )
    .err()
    .unwrap();
    assert!(error.started);
    assert!(error.message.contains("output"), "{}", error.message);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("marker")).unwrap(),
        "executed"
    );
    let logs = store.create().unwrap();
    let error = run_logged(
        &mut Command::new(directory.path().join("missing-executable")),
        None,
        4096,
        &Cancellation::default(),
        [logs.stdout, logs.stderr],
    )
    .err()
    .unwrap();
    assert!(!error.started);
}
