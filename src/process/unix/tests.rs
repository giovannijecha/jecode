use crate::{cancel::Cancellation, process, test_support::Directory};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const OWNER: &str = "process::unix::tests::owner_exit_fixture";

#[test]
fn owner_exit_fixture() {
    let Some(mode) = std::env::var_os("JECODE_UNIX_OWNER_FIXTURE") else {
        return;
    };
    let mut command = Command::new(crate::tools::find_bash().unwrap());
    command
        .args(["--noprofile", "--norc", "-c", "printf '%s' \"$$\" > group; (sleep 1.5; printf survived > survivor) >/dev/null 2>&1 & printf 'started\\n'; sleep 20"])
        .current_dir(std::env::var_os("JECODE_UNIX_OWNER_DIRECTORY").unwrap());
    process::run_observed(
        &mut command,
        None,
        None,
        4096,
        &Cancellation::default(),
        None,
        &mut |bytes| {
            if mode == "exit" && bytes.ends_with(b"started\n") {
                std::process::exit(0);
            }
            Ok(())
        },
    )
    .unwrap();
}

struct OwnerFixture<'a> {
    child: Child,
    directory: &'a Directory,
}

impl Drop for OwnerFixture<'_> {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Also clean the regression's group if the assertion fails on old code.
        if let Ok(group) = std::fs::read_to_string(self.directory.path().join("group")) {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", group.trim())])
                .stderr(Stdio::null())
                .status();
        }
    }
}

fn owner_disappears(mode: &str) {
    let directory = Directory::new();
    let mut owner = OwnerFixture {
        child: Command::new(std::env::current_exe().unwrap())
            .args(["--exact", OWNER, "--nocapture"])
            .env("JECODE_UNIX_OWNER_FIXTURE", mode)
            .env("JECODE_UNIX_OWNER_DIRECTORY", directory.path())
            .env_remove("BASH_ENV")
            .env_remove("ENV")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        directory: &directory,
    };
    let started = Instant::now();
    while !directory.path().join("group").exists() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "target did not start"
        );
        thread::sleep(Duration::from_millis(10));
    }
    if mode == "kill" {
        owner.child.kill().unwrap();
    }
    while owner.child.try_wait().unwrap().is_none() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "owner did not exit"
        );
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(1700));
    assert!(
        !directory.path().join("survivor").exists(),
        "command survived its owner"
    );
}

#[test]
fn owner_exit_without_dropping_state_stops_commands() {
    owner_disappears("exit");
}

#[test]
fn owner_sigkill_stops_commands() {
    owner_disappears("kill");
}

#[test]
fn binary_input_arguments_environment_and_exit_status_survive_the_boundary() {
    let directory = Directory::new();
    let mut command = Command::new(crate::tools::find_bash().unwrap());
    command
        .args([
            "--noprofile",
            "--norc",
            "-c",
            "cat; printf '%s\\n' \"$1\" \"$FIXTURE_VALUE\"; printf err >&2; exit 17",
            "fixture",
            "\"quoted\" $literal\nline",
        ])
        .env("FIXTURE_VALUE", "hello λ")
        .current_dir(directory.path());
    let mut expected = vec![0, 255, 10, 13];
    expected.extend(std::iter::repeat_n(b'x', 256 * 1024));
    let input = expected.clone();
    expected.extend_from_slice("\"quoted\" $literal\nline\nhello λ\n".as_bytes());
    let output = process::run_observed(
        &mut command,
        Some(input),
        Duration::from_secs(5),
        expected.len(),
        &Cancellation::default(),
        None,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(output.stdout, expected);
    assert_eq!(output.stderr, b"err");
    assert_eq!(output.exit_code, Some(17));
    assert!(!output.timed_out && !output.cancelled);
}

#[test]
fn blocked_input_does_not_delay_cancellation() {
    let directory = Directory::new();
    let mut command = Command::new(crate::tools::find_bash().unwrap());
    command
        .args(["--noprofile", "--norc", "-c", "printf ready; sleep 20"])
        .current_dir(directory.path());
    let cancellation = Cancellation::default();
    let signal = cancellation.clone();
    let started = Instant::now();
    let output = process::run_observed(
        &mut command,
        Some(vec![0; 1024 * 1024]),
        None,
        4096,
        &cancellation,
        None,
        &mut |_| {
            signal.cancel();
            Ok(())
        },
    )
    .unwrap();
    assert!(output.cancelled);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn input_eof_arrives_while_the_owner_remains_alive() {
    let mut command = Command::new("cat");
    let output = process::run_observed(
        &mut command,
        Some(Vec::new()),
        Duration::from_secs(2),
        4096,
        &Cancellation::default(),
        None,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert!(!output.timed_out);
    assert!(output.stdout.is_empty());
}

#[test]
fn missing_executable_is_reported_before_execution() {
    let directory = Directory::new();
    let error = super::spawn(
        &Command::new(directory.path().join("absent")),
        Some(vec![0; 8192]),
        &Cancellation::default(),
    )
    .err()
    .unwrap();
    assert!(!error.started);
    assert!(error.message.contains("Could not start process"));
}

#[test]
fn inherited_shell_tracing_cannot_print_target_arguments() {
    let mut command = Command::new("printf");
    command
        .args(["%s", "fixture-private-value"])
        .env("SHELLOPTS", "xtrace");
    let output = process::run_observed(
        &mut command,
        None,
        Duration::from_secs(2),
        4096,
        &Cancellation::default(),
        None,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(output.stdout, b"fixture-private-value");
    assert!(output.stderr.is_empty(), "Shell tracing reached stderr");
}
