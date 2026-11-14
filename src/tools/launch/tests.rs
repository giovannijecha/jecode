use super::*;
use crate::permissions::Level;
use crate::test_support::Directory;
use crate::tools::{Begun, Tools};
use std::path::Path;

const FIXTURE: &str = "tools::launch::tests::launch_fixture";
#[cfg(windows)]
const JOB_FIXTURE: &str = "tools::launch::tests::restrictive_job_fixture";
#[cfg(windows)]
const JOB_FIXTURE_ENV: &str = "JECODE_LAUNCH_RESTRICTIVE_JOB_FIXTURE";

/// A disposable application: it beats until a `stop` file appears, and never
/// longer than 30 seconds, so a failed test still leaves nothing running.
#[test]
fn launch_fixture() {
    let Some(directory) = std::env::var_os("JECODE_LAUNCH_FIXTURE") else {
        return;
    };
    if std::env::var_os("JECODE_LAUNCH_EXIT").is_some() {
        std::process::exit(9);
    }
    let directory = std::path::PathBuf::from(directory);
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(30) && !directory.join("stop").exists() {
        use std::io::Write;
        let mut beat = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.join("heartbeat"))
            .unwrap();
        beat.write_all(b"x").unwrap();
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// This test body runs only when the outer test starts it through the owned
/// Windows supervisor, whose Job Object has no breakaway permission.
#[cfg(windows)]
#[test]
fn restrictive_job_fixture() {
    let Some(directory) = std::env::var_os(JOB_FIXTURE_ENV) else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    let workspace = Directory::new();
    let tools = Tools::new(workspace.path()).unwrap();
    let result = exec(
        &tools,
        Some(Level::Full),
        &arguments(&directory, &[]),
        &Cancellation::default(),
    );
    let error = result
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        error.contains("independently of Jecode's host"),
        "{result:?}"
    );
    assert!(error.contains("Nothing was launched"), "{result:?}");
    if std::env::var_os("JECODE_LAUNCH_EXPECT_NESTED").is_some() {
        assert!(
            error.contains("host kept the launch helper inside a Job Object"),
            "breakaway must succeed past the inner job before the ancestor blocks it: {result:?}"
        );
    }
    assert_eq!(beats(&directory), 0, "a refused application ran");
    println!("restrictive job refusal verified");
}

#[cfg(windows)]
#[test]
fn a_restrictive_job_refuses_persistent_breakaway_without_starting_the_application() {
    let fixture = Directory::new();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", JOB_FIXTURE, "--nocapture"])
        .env(JOB_FIXTURE_ENV, fixture.path());
    let output = crate::process::run_observed(
        &mut command,
        None,
        Duration::from_secs(15),
        64 * 1024,
        &Cancellation::default(),
        None,
        &mut |_| Ok(()),
    );
    // If a regression accidentally starts the application, it has a stop
    // signal even when the child test fails. It also has its own 30 s cap.
    std::fs::write(fixture.path().join("stop"), "").unwrap();
    let output = output.unwrap();
    assert_eq!(
        output.exit_code,
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.timed_out && !output.cancelled);
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("restrictive job refusal verified"),
        "the inner fixture did not run: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(beats(fixture.path()), 0, "a refused application ran");
}

#[cfg(windows)]
#[test]
#[cfg_attr(
    windows,
    ignore = "requires a host permitting independent process creation"
)]
fn an_outer_job_that_forbids_breakaway_refuses_the_app_before_it_runs() {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    let fixture = Directory::new();
    let helper = crate::native::helper("launch").unwrap();
    let executable = std::env::current_exe().unwrap();
    let mut command = std::process::Command::new(helper);
    command
        .arg("--nested-test")
        .arg(executable)
        .arg(JOB_FIXTURE)
        .env(JOB_FIXTURE_ENV, fixture.path())
        .env("JECODE_LAUNCH_EXPECT_NESTED", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x0100_0000 | 0x0800_0000);
    let mut child = command.spawn().unwrap();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > Duration::from_secs(25) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("nested Job Object launch fixture timed out");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    std::fs::write(fixture.path().join("stop"), "").unwrap();
    let mut stdout = String::new();
    let mut stderr = String::new();
    use std::io::Read;
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(status.success(), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("nested job fixture exited 0"), "{stdout}");
    assert!(
        stdout.contains("restrictive job refusal verified"),
        "{stdout}"
    );
    assert_eq!(beats(fixture.path()), 0, "a refused application ran");
}

fn arguments(directory: &Path, extra: &[(&str, Value)]) -> Value {
    let mut arguments = Value::object([
        (
            "executable",
            Value::string(std::env::current_exe().unwrap().to_string_lossy()),
        ),
        (
            "args",
            Value::Array(vec![Value::string("--exact"), Value::string(FIXTURE)]),
        ),
        (
            "env",
            Value::object([(
                "JECODE_LAUNCH_FIXTURE",
                Value::string(directory.to_string_lossy()),
            )]),
        ),
        ("persistent", Value::Bool(true)),
        ("wait_ms", Value::number(300)),
    ]);
    if let Value::Object(fields) = &mut arguments {
        for (key, value) in extra {
            fields.insert((*key).into(), value.clone());
        }
    }
    arguments
}

fn exec(
    tools: &Tools,
    level: Option<Level>,
    arguments: &Value,
    cancellation: &Cancellation,
) -> Value {
    let begun = level.map_or_else(Begun::default, |level| {
        Begun::default().with_permission(level, None)
    });
    tools
        .execute_begun(begun, "exec", &arguments.encode(), cancellation)
        .result
}

fn beats(directory: &Path) -> u64 {
    std::fs::metadata(directory.join("heartbeat")).map_or(0, |metadata| metadata.len())
}

fn beating(directory: &Path) -> bool {
    let before = beats(directory);
    std::thread::sleep(Duration::from_millis(400));
    beats(directory) > before
}

/// Ends the fixture and confirms it stopped.
fn finish(directory: &Path) {
    std::fs::write(directory.join("stop"), "").unwrap();
    let started = Instant::now();
    while beating(directory) {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "fixture did not stop"
        );
    }
}

fn wait_for_beats(directory: &Path) {
    let started = Instant::now();
    while beats(directory) == 0 {
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "fixture did not start"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[cfg_attr(
    windows,
    ignore = "requires a host permitting independent process creation"
)]
fn a_persistent_application_survives_the_call_and_the_conversation() {
    let workspace = Directory::new();
    let fixture = Directory::new();
    let tools = Tools::new(workspace.path()).unwrap();
    let result = exec(
        &tools,
        Some(Level::Full),
        &arguments(fixture.path(), &[]),
        &Cancellation::default(),
    );
    assert_eq!(
        result.get("launch").and_then(Value::as_str),
        Some("persistent"),
        "{result:?}"
    );
    assert_eq!(
        result.get("application").and_then(Value::as_str),
        Some("running"),
        "{result:?}"
    );
    assert!(
        result
            .get("pid")
            .and_then(Value::as_usize)
            .is_some_and(|pid| pid > 0)
    );
    assert!(result.get("exit_code").is_none());
    assert!(
        result.get("process_id").is_none(),
        "Jecode does not own it: {result:?}"
    );
    let listed = tools.execute("process", r#"{"action":"list"}"#);
    assert_eq!(listed.get("processes"), Some(&Value::Array(Vec::new())));
    wait_for_beats(fixture.path());
    // The conversation ends: owned processes stop, this application does not.
    drop(tools);
    assert!(
        beating(fixture.path()),
        "persistent application ended with the conversation"
    );
    finish(fixture.path());
}

#[test]
fn an_ordinary_launch_of_the_same_application_ends_with_the_conversation() {
    let workspace = Directory::new();
    let fixture = Directory::new();
    let tools = Tools::new(workspace.path()).unwrap();
    let mut ordinary = arguments(fixture.path(), &[]);
    if let Value::Object(fields) = &mut ordinary {
        fields.remove("persistent");
        fields.insert("wait_ms".into(), Value::number(0));
    }
    let result = exec(
        &tools,
        Some(Level::Full),
        &ordinary,
        &Cancellation::default(),
    );
    assert_eq!(
        result.get("status").and_then(Value::as_str),
        Some("running"),
        "{result:?}"
    );
    wait_for_beats(fixture.path());
    drop(tools);
    assert!(
        !beating(fixture.path()),
        "an owned process survived the conversation"
    );
}

#[test]
#[cfg_attr(
    windows,
    ignore = "requires a host permitting independent process creation"
)]
fn an_application_that_exits_while_observed_reports_its_exit_not_a_launch() {
    let workspace = Directory::new();
    let fixture = Directory::new();
    let tools = Tools::new(workspace.path()).unwrap();
    let mut failing = arguments(fixture.path(), &[("wait_ms", Value::number(10_000))]);
    if let Value::Object(fields) = &mut failing
        && let Some(Value::Object(env)) = fields.get_mut("env")
    {
        env.insert("JECODE_LAUNCH_EXIT".into(), Value::string("1"));
    }
    let started = Instant::now();
    let result = exec(
        &tools,
        Some(Level::Full),
        &failing,
        &Cancellation::default(),
    );
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "observation must end at exit"
    );
    assert_eq!(
        result.get("application").and_then(Value::as_str),
        Some("exited"),
        "{result:?}"
    );
    assert_eq!(result.get("exit_code"), Some(&Value::number(9)));
}

#[test]
fn only_full_authorizes_a_persistent_launch_and_nothing_runs_otherwise() {
    let workspace = Directory::new();
    let fixture = Directory::new();
    let tools = Tools::new(workspace.path()).unwrap();
    for level in [None, Some(Level::Workspace)] {
        let result = exec(
            &tools,
            level,
            &arguments(fixture.path(), &[]),
            &Cancellation::default(),
        );
        let error = result
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(
            error.contains("need the Full permission level"),
            "{level:?}: {result:?}"
        );
    }
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(beats(fixture.path()), 0, "a refused launch ran");
}

#[test]
fn arguments_that_imply_ownership_are_rejected_before_launch() {
    let workspace = Directory::new();
    let fixture = Directory::new();
    let tools = Tools::new(workspace.path()).unwrap();
    for (name, value, expected) in [
        ("tty", Value::Bool(true), "tty does not apply"),
        ("stdin", Value::string("x"), "stdin does not apply"),
        (
            "timeout_seconds",
            Value::number(5),
            "timeout_seconds does not apply",
        ),
        (
            "output_limit_bytes",
            Value::number(5),
            "output_limit_bytes does not apply",
        ),
        (
            "persistent",
            Value::string("yes"),
            "persistent must be a boolean",
        ),
    ] {
        let result = exec(
            &tools,
            Some(Level::Full),
            &arguments(fixture.path(), &[(name, value)]),
            &Cancellation::default(),
        );
        let error = result
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(error.contains(expected), "{name}: {result:?}");
    }
    let missing = exec(
        &tools,
        Some(Level::Full),
        &Value::object([
            ("executable", Value::string("jecode-no-such-application")),
            ("persistent", Value::Bool(true)),
        ]),
        &Cancellation::default(),
    );
    let error = missing
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        error.to_ascii_lowercase().contains("nothing was launched"),
        "{missing:?}"
    );
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(beats(fixture.path()), 0, "a rejected launch ran");
}

#[test]
fn a_cancelled_request_never_starts_a_persistent_application() {
    let workspace = Directory::new();
    let fixture = Directory::new();
    let tools = Tools::new(workspace.path()).unwrap();
    let before = Cancellation::default();
    before.cancel();
    let refused = exec(
        &tools,
        Some(Level::Full),
        &arguments(fixture.path(), &[]),
        &before,
    );
    assert!(
        refused
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("nothing was launched"),
        "{refused:?}"
    );
    assert_eq!(beats(fixture.path()), 0, "a cancelled launch ran");
}

#[test]
#[cfg_attr(
    windows,
    ignore = "requires a host permitting independent process creation"
)]
fn cancellation_ends_only_the_observation_once_the_application_started() {
    let workspace = Directory::new();
    let fixture = Directory::new();
    let tools = Tools::new(workspace.path()).unwrap();
    let cancellation = Cancellation::default();
    let signal = cancellation.clone();
    let watched = fixture.path().to_path_buf();
    let canceller = std::thread::spawn(move || {
        wait_for_beats(&watched);
        signal.cancel();
    });
    let result = exec(
        &tools,
        Some(Level::Full),
        &arguments(fixture.path(), &[("wait_ms", Value::number(30_000))]),
        &cancellation,
    );
    canceller.join().unwrap();
    assert_eq!(
        result.get("wait_cancelled"),
        Some(&Value::Bool(true)),
        "{result:?}"
    );
    assert_eq!(
        result.get("application").and_then(Value::as_str),
        Some("running")
    );
    assert!(
        beating(fixture.path()),
        "cancellation stopped a persistent application"
    );
    finish(fixture.path());
}
