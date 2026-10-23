use super::*;
use crate::redact::Redactor;
use crate::test_support::Directory;
use crate::tools::Tools;

// The root exits at once; the descendant keeps the boundary alive.
const HEARTBEAT: &str = "(while :; do printf x >> heartbeat; sleep 0.05; done) & printf 'ready\\n'";

fn tools(directory: &Directory, redactor: Redactor) -> Tools {
    let mut tools = Tools::new(directory.path()).unwrap();
    tools.configure_output(directory.path().join("logs"), redactor);
    tools
}

fn bash(tools: &Tools, command: &str, extra: &[(&str, Value)]) -> Value {
    let mut arguments = Value::object([("command", Value::string(command))]);
    if let Value::Object(fields) = &mut arguments {
        for (key, value) in extra {
            fields.insert((*key).into(), value.clone());
        }
    }
    tools.execute("bash", &arguments.encode())
}

fn start(tools: &Tools, command: &str) -> (String, Value) {
    let result = bash(tools, command, &[("wait_ms", Value::number(0))]);
    let id = text(&result, "process_id");
    assert!(id.starts_with("process-"), "{result:?}");
    (id, result)
}

fn process(tools: &Tools, action: &str, id: &str, wait: Option<usize>) -> Value {
    process_with(tools, action, id, wait, &Cancellation::default())
}

fn process_with(
    tools: &Tools,
    action: &str,
    id: &str,
    wait: Option<usize>,
    cancellation: &Cancellation,
) -> Value {
    let mut arguments = Value::object([
        ("action", Value::string(action)),
        ("process_id", Value::string(id)),
    ]);
    if let (Some(wait), Value::Object(fields)) = (wait, &mut arguments) {
        fields.insert("wait_ms".into(), Value::number(wait));
    }
    tools.execute_with_cancel("process", &arguments.encode(), cancellation)
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{key} missing from {value:?}"))
        .into()
}

fn number(value: &Value, key: &str) -> usize {
    value.get(key).and_then(Value::as_usize).unwrap()
}

fn release(directory: &Directory, gate: &str) {
    std::fs::write(directory.path().join(gate), "").unwrap();
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "condition timed out"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn heartbeat_stopped(directory: &Directory) -> bool {
    let size = || {
        std::fs::metadata(directory.path().join("heartbeat")).map_or(0, |metadata| metadata.len())
    };
    let before = size();
    thread::sleep(Duration::from_millis(400));
    before == size()
}

fn gate(name: &str) -> String {
    format!("until [ -e {name} ]; do sleep 0.02; done")
}

#[test]
fn persistent_bash_reports_only_its_prelaunch_capture() {
    use crate::checkpoints::session::Session;

    let home = Directory::new();
    let root = Directory::new();
    let session = Session::open(home.path(), root.path());
    session.begin("model");
    let mut tools = tools(&root, Redactor::empty());
    tools.configure_checkpoints(std::sync::Arc::clone(&session));
    let started = bash(
        &tools,
        &format!(
            "{}; mkdir -p generated; printf final > generated/result.txt",
            gate("go")
        ),
        &[
            ("wait_ms", Value::number(0)),
            ("protect", Value::Array(vec![Value::string("generated/")])),
        ],
    );
    assert_eq!(text(&started, "status"), "running");
    let checkpoint = started.get("checkpoint").unwrap();
    assert_eq!(
        checkpoint.get("status").and_then(Value::as_str),
        Some("saved")
    );
    assert!(text(checkpoint, "ref").starts_with("rc-"));
    release(&root, "go");
    let finished = process(&tools, "poll", &text(&started, "process_id"), Some(10_000));
    assert_eq!(text(&finished, "status"), "exited");
    assert!(finished.get("checkpoint_finalized").is_none());
    assert!(finished.get("checkpoint_error").is_none());
    assert_eq!(
        std::fs::read(root.path().join("generated/result.txt")).unwrap(),
        b"final"
    );
}

#[test]
fn a_running_process_returns_control_and_later_reports_all_output_and_its_exit() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let (id, first) = start(
        &tools,
        &format!(
            "printf 'first\\n'; {}; printf 'last\\n'; exit 3",
            gate("go")
        ),
    );
    assert_eq!(text(&first, "status"), "running");
    assert!(first.get("exit_code").is_none(), "{first:?}");
    // Another tool runs while the process remains alive.
    let write = tools.execute(
        "write",
        &Value::object([
            ("path", Value::string("go")),
            ("content", Value::string("")),
        ])
        .encode(),
    );
    assert!(write.get("error").is_none(), "{write:?}");
    let last = process(&tools, "poll", &id, Some(10_000));
    assert_eq!(text(&last, "status"), "exited");
    assert_eq!(number(&last, "exit_code"), 3);
    assert_eq!(
        text(&first, "stdout") + &text(&last, "stdout"),
        "first\nlast\n"
    );
    assert_eq!(
        number(&last, "stdout_offset"),
        number(&first, "stdout_next_offset")
    );
    assert_eq!(number(&last, "stdout_next_offset"), 11);
    assert_eq!(number(&last, "stdout_skipped_bytes"), 0);
    let reread = tools.execute(
        "read",
        &Value::object([("path", Value::string(text(&last, "stdout_file")))]).encode(),
    );
    assert!(text(&reread, "content").contains("first"), "{reread:?}");
    // A finished process stays inspectable without repeating delivered output.
    let again = process(&tools, "poll", &id, None);
    assert_eq!(text(&again, "status"), "exited");
    assert_eq!(text(&again, "stdout"), "");
    assert_eq!(number(&again, "exit_code"), 3);
}

#[test]
fn output_between_polls_and_immediately_before_exit_is_retained() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let (id, first) = start(
        &tools,
        &format!(
            "printf 'a\\n'; {}; printf 'b\\n' >&2; printf 'b\\n'; {}; printf 'c'",
            gate("one"),
            gate("two")
        ),
    );
    let mut stdout = text(&first, "stdout");
    let mut stderr = text(&first, "stderr");
    release(&directory, "one");
    wait_for(|| {
        let middle = process(&tools, "poll", &id, None);
        assert_eq!(text(&middle, "status"), "running");
        stdout += &text(&middle, "stdout");
        stderr += &text(&middle, "stderr");
        stdout == "a\nb\n" && stderr == "b\n"
    });
    release(&directory, "two");
    // The process exits between polls; its final output remains readable.
    wait_for(|| tools.processes.list().encode().contains("exited"));
    let last = process(&tools, "poll", &id, None);
    assert_eq!(text(&last, "status"), "exited");
    assert_eq!(stdout + &text(&last, "stdout"), "a\nb\nc");
    assert_eq!(text(&last, "stderr"), "");
}

#[test]
fn multiple_processes_are_independently_addressable() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let (one, _) = start(&tools, &format!("{}; printf one", gate("first")));
    let (two, _) = start(&tools, &format!("{}; printf two", gate("second")));
    assert_ne!(one, two);
    let listed = tools.execute("process", r#"{"action":"list"}"#);
    let processes = listed.get("processes").and_then(Value::as_array).unwrap();
    assert_eq!(processes.len(), 2);
    assert!(
        processes
            .iter()
            .all(|process| text(process, "status") == "running")
    );
    let stopped = process(&tools, "stop", &one, None);
    assert_eq!(text(&stopped, "status"), "stopped");
    assert!(stopped.get("exit_code").is_none());
    release(&directory, "second");
    let finished = process(&tools, "poll", &two, Some(10_000));
    assert_eq!(text(&finished, "status"), "exited");
    assert_eq!(text(&finished, "stdout"), "two");
    assert_eq!(
        text(&process(&tools, "poll", &one, None), "status"),
        "stopped"
    );
}

#[test]
fn wait_expiry_keeps_the_process_and_the_execution_timeout_needs_no_polling() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let started = Instant::now();
    let waited = bash(&tools, &gate("never"), &[("wait_ms", Value::number(300))]);
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert_eq!(text(&waited, "status"), "running");
    let id = text(&waited, "process_id");
    thread::sleep(Duration::from_millis(300));
    assert_eq!(
        text(&process(&tools, "poll", &id, None), "status"),
        "running"
    );
    process(&tools, "stop", &id, None);

    let started = Instant::now();
    let limited = bash(
        &tools,
        &format!("printf started; {}", gate("never")),
        &[
            ("wait_ms", Value::number(0)),
            ("timeout_seconds", Value::number(1)),
        ],
    );
    let id = text(&limited, "process_id");
    wait_for(|| tools.processes.list().encode().contains("timed_out"));
    assert!(started.elapsed() < Duration::from_secs(8));
    let result = process(&tools, "poll", &id, None);
    assert_eq!(text(&result, "status"), "timed_out");
    assert!((text(&limited, "stdout") + &text(&result, "stdout")).contains("started"));
}

#[test]
fn cancelling_a_wait_preserves_the_process() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let cancellation = Cancellation::default();
    let started = Instant::now();
    let launched = thread::scope(|scope| {
        // Interrupt only the wait: the process is registered once launched.
        scope.spawn(|| {
            wait_for(|| !tools.processes.scope().entries.lock().unwrap().is_empty());
            thread::sleep(Duration::from_millis(100));
            cancellation.cancel();
        });
        tools.execute_with_cancel(
            "bash",
            &Value::object([
                (
                    "command",
                    Value::string(format!("{}; printf done", gate("go"))),
                ),
                ("wait_ms", Value::number(30_000)),
            ])
            .encode(),
            &cancellation,
        )
    });
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(launched.get("wait_cancelled"), Some(&Value::Bool(true)));
    assert_eq!(text(&launched, "status"), "running");
    let id = text(&launched, "process_id");
    let polled = process_with(&tools, "poll", &id, Some(30_000), &cancellation);
    assert_eq!(polled.get("wait_cancelled"), Some(&Value::Bool(true)));
    assert_eq!(text(&polled, "status"), "running");
    release(&directory, "go");
    let finished = process(&tools, "poll", &id, Some(10_000));
    assert_eq!(text(&finished, "status"), "exited");
    assert_eq!(text(&finished, "stdout"), "done");
}

#[test]
fn launch_cancelled_before_start_reports_that_nothing_ran() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let result = tools.execute_with_cancel(
        "bash",
        r#"{"command":"printf ran > marker","wait_ms":0}"#,
        &cancellation,
    );
    assert_eq!(text(&result, "error"), "Operation cancelled");
    assert!(!directory.path().join("marker").exists());
    let listed = tools.execute("process", r#"{"action":"list"}"#);
    assert_eq!(
        listed
            .get("processes")
            .and_then(Value::as_array)
            .map(<[_]>::len),
        Some(0)
    );
}

#[test]
fn stop_terminates_descendants() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let (id, _) = start(&tools, HEARTBEAT);
    wait_for(|| directory.path().join("heartbeat").exists());
    assert_eq!(
        text(&process(&tools, "poll", &id, Some(200)), "status"),
        "running"
    );
    let stopped = process(&tools, "stop", &id, None);
    assert_eq!(text(&stopped, "status"), "stopped");
    assert!(heartbeat_stopped(&directory));
}

#[test]
fn conversation_and_application_cleanup_terminate_descendants() {
    for drop_tools in [false, true] {
        let directory = Directory::new();
        let tools = tools(&directory, Redactor::empty());
        let (id, _) = start(&tools, HEARTBEAT);
        wait_for(|| directory.path().join("heartbeat").exists());
        if drop_tools {
            drop(tools);
            assert!(heartbeat_stopped(&directory));
            continue;
        }
        tools.stop_processes();
        assert!(heartbeat_stopped(&directory));
        // The ended conversation's ID never addresses a later process.
        let stale = process(&tools, "poll", &id, None);
        assert!(
            text(&stale, "error").contains("never restarted"),
            "{stale:?}"
        );
        let (next, _) = start(&tools, "true");
        assert_ne!(next, id);
    }
}

#[test]
fn finished_and_unknown_references_behave_predictably() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let finished = bash(&tools, "printf done", &[("wait_ms", Value::number(10_000))]);
    assert_eq!(text(&finished, "status"), "exited");
    assert_eq!(number(&finished, "exit_code"), 0);
    let id = text(&finished, "process_id");
    let stopped = process(&tools, "stop", &id, None);
    assert_eq!(text(&stopped, "status"), "exited");
    assert_eq!(text(&stopped, "stdout"), "");
    let unknown = process(&tools, "poll", "process-1-2-3", None);
    assert!(text(&unknown, "error").contains("process-1-2-3"));
    let invalid = tools.execute("process", r#"{"action":"restart","process_id":"x"}"#);
    assert!(text(&invalid, "error").contains("list, poll or stop"));
    let negative = bash(&tools, "true", &[("wait_ms", Value::number(-1))]);
    assert!(text(&negative, "error").contains("wait_ms"));
}

#[test]
fn live_reads_mask_credentials_split_across_chunks() {
    let directory = Directory::new();
    let secret = "fixture-secret-value";
    let tools = tools(&directory, Redactor::new(secret.into()));
    let (id, _) = start(
        &tools,
        &format!(
            "printf 'ready fixture-sec'; {}; printf 'ret-value done'",
            gate("go")
        ),
    );
    let mut stdout = String::new();
    wait_for(|| {
        stdout += &text(&process(&tools, "poll", &id, None), "stdout");
        stdout == "ready "
    });
    release(&directory, "go");
    let last = process(&tools, "poll", &id, Some(10_000));
    stdout += &text(&last, "stdout");
    assert_eq!(stdout, "ready [redacted] done");
    let saved =
        std::fs::read_to_string(tools.outputs.resolve(&text(&last, "stdout_file")).unwrap());
    assert_eq!(saved.unwrap(), "ready [redacted] done");
}

#[test]
fn bounded_reads_respect_utf8_boundaries_and_report_skipped_bytes() {
    let directory = Directory::new();
    let path = directory.path().join("stream");
    // One partial character at the end of a live stream waits for the next read.
    std::fs::write(&path, &"aé".as_bytes()[..2]).unwrap();
    let live = read_after(&path, 0, false).unwrap();
    assert_eq!((live.text.as_str(), live.next), ("a", 1));
    let finished = read_after(&path, 1, true).unwrap();
    assert_eq!(finished.text, "\u{fffd}");
    // A tail cut never begins inside a character.
    let mut bytes = "é".repeat(OUTPUT_LIMIT).into_bytes();
    bytes.truncate(OUTPUT_LIMIT + 101);
    std::fs::write(&path, &bytes).unwrap();
    let tail = read_after(&path, 0, false).unwrap();
    assert_eq!(tail.offset, 102);
    assert_eq!(tail.next, bytes.len() as u64 - 1);
    assert!(tail.text.chars().all(|character| character == 'é'));
}

#[test]
fn a_failed_stream_read_preserves_both_output_cursors() {
    let directory = Directory::new();
    let stdout = directory.path().join("stdout");
    let stderr = directory.path().join("stderr");
    std::fs::write(&stdout, "unread stdout").unwrap();
    let entry = Entry {
        id: "process-fixture".into(),
        command: "fixture".into(),
        streams: [
            Stream {
                reference: "stdout".into(),
                path: stdout,
            },
            Stream {
                reference: "stderr".into(),
                path: stderr.clone(),
            },
        ],
        stop: Cancellation::default(),
        state: Mutex::new(State {
            outcome: Some(Outcome {
                status: "exited",
                exit_code: Some(0),
                error: None,
            }),
            cursors: [0, 0],
        }),
        changed: Condvar::new(),
        monitor: Mutex::new(None),
    };
    assert!(entry.report(false).unwrap_err().contains("saved stderr"));
    std::fs::write(stderr, "unread stderr").unwrap();
    let recovered = entry.report(false).unwrap();
    assert_eq!(text(&recovered, "stdout"), "unread stdout");
    assert_eq!(text(&recovered, "stderr"), "unread stderr");
    assert_eq!(number(&recovered, "stdout_offset"), 0);
    assert_eq!(number(&recovered, "stderr_offset"), 0);
}

#[test]
fn foreground_bash_without_wait_keeps_its_result_shape() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let result = bash(&tools, "printf out; exit 4", &[]);
    assert!(result.get("process_id").is_none());
    assert_eq!(number(&result, "exit_code"), 4);
    assert_eq!(text(&result, "capture"), "tail");
    let listed = tools.execute("process", r#"{"action":"list"}"#);
    assert_eq!(
        listed
            .get("processes")
            .and_then(Value::as_array)
            .map(<[_]>::len),
        Some(0)
    );
}

mod observer_tests;
