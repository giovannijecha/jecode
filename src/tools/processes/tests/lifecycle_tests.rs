//! Terminal transitions: delivery, races, masking, retention and cleanup.

use super::wait_tests::{observer, only};
use super::*;
use std::sync::atomic::AtomicBool;

#[test]
fn completion_is_announced_once_after_final_output_is_available() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let observer = observer(&tools);
    let (id, started) = start(
        &tools,
        &format!(
            "printf out; printf err >&2; {}; printf last; exit 5",
            gate("go")
        ),
    );
    assert!(observer.completions_since(0).1.is_empty());
    assert!(!tools.process_completion_waiting());
    release(&directory, "go");
    wait_for(|| tools.process_completion_waiting());

    // An interface and the model each read from their own position.
    let (cursor, seen) = observer.completions_since(0);
    assert_eq!(seen.len(), 1);
    assert_eq!(observer.completions_since(cursor).1.len(), 0);
    let completion = &seen[0];
    assert_eq!(completion.id, id);
    assert_eq!(completion.status, "exited");
    assert_eq!(completion.exit_code, Some(5));
    assert_eq!(completion.tails[0], "outlast");
    assert_eq!(completion.tails[1], "err");
    assert_eq!(
        completion.command,
        format!(
            "printf out; printf err >&2; {}; printf last; exit 5",
            gate("go")
        )
    );
    assert!(tools.outputs.resolve(&completion.stdout_file).is_ok());
    assert!(tools.outputs.resolve(&completion.stderr_file).is_ok());
    assert!(!completion.stop_requested);
    // Saved output is complete by the time the event exists.
    assert_eq!(
        std::fs::read_to_string(tools.outputs.resolve(&completion.stdout_file).unwrap()).unwrap(),
        "outlast"
    );

    assert_eq!(tools.process_completions(&Redactor::empty()).len(), 1);
    assert!(
        tools.process_completion_waiting(),
        "peeking delivers nothing"
    );
    tools.processes_reported(std::slice::from_ref(&id));
    assert!(tools.process_completions(&Redactor::empty()).is_empty());
    assert!(!tools.process_completion_waiting());
    // The model's cursor and the interface's event are untouched by delivery.
    let rest = process(&tools, "poll", &id, None);
    assert_eq!(text(&started, "stdout") + &text(&rest, "stdout"), "outlast");
    assert_eq!(observer.completions_since(0).1.len(), 1);
}

#[cfg(unix)]
#[test]
fn a_signaled_command_reports_an_exit_without_an_exit_code() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let launched = tools.execute(
        "exec",
        &Value::object([
            ("executable", Value::string("/bin/sh")),
            (
                "args",
                Value::Array(vec![
                    Value::string("-c"),
                    Value::string("until [ -f go ]; do :; done; kill -TERM $$"),
                ]),
            ),
            ("wait_ms", Value::number(0)),
            ("timeout_seconds", Value::number(15)),
        ])
        .encode(),
    );
    assert_eq!(text(&launched, "status"), "running");
    let id = text(&launched, "process_id");
    release(&directory, "go");
    wait_for(|| tools.process_completion_waiting());

    let completions = tools.process_completions(&Redactor::empty());
    assert_eq!(completions.len(), 1);
    let completion = &completions[0];
    assert_eq!(completion.id, id);
    assert_eq!(completion.status, "exited");
    assert_eq!(completion.exit_code, None);
    assert!(!completion.stop_requested);
    assert_eq!(completion.value().get("exit_code"), Some(&Value::Null));
    assert!(completion.report().contains("exited with no exit code"));
}

#[test]
fn a_poll_or_list_that_reports_the_outcome_replaces_its_delivery() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let (first, _) = start(&tools, &gate("one"));
    release(&directory, "one");
    let polled = process(&tools, "poll", &first, Some(10_000));
    assert_eq!(text(&polled, "status"), "exited");
    assert!(!tools.process_completion_waiting());
    assert!(tools.process_completions(&Redactor::empty()).is_empty());

    let (second, _) = start(&tools, &gate("two"));
    release(&directory, "two");
    wait_for(|| tools.process_completion_waiting());
    tools.execute("process", r#"{"action":"list"}"#);
    assert!(!tools.process_completion_waiting());
    assert!(tools.process_completions(&Redactor::empty()).is_empty());
    // Already told through the list; a poll can still read its output.
    assert_eq!(
        text(&process(&tools, "poll", &second, None), "status"),
        "exited"
    );
}

#[test]
fn a_completion_racing_the_launch_receipt_is_delivered_exactly_once() {
    // The command ends around the wait's end; either the receipt or the
    // announcement carries it, never both and never neither.
    for wait in [0, 1, 2, 5, 10, 20] {
        let directory = Directory::new();
        let tools = tools(&directory, Redactor::empty());
        tools.set_default_wait(Duration::from_millis(wait));
        let result = bash(&tools, "printf x", &[]);
        if result.get("process_id").is_some() {
            let observer = observer(&tools);
            wait_for(|| !only(&observer).active());
            let announced = tools.process_completions(&Redactor::empty()).len();
            if text(&result, "status") == "exited" {
                assert_eq!(announced, 0, "the receipt carried it; wait {wait}");
            } else {
                assert_eq!(announced, 1, "only an announcement can; wait {wait}");
            }
        } else {
            assert_eq!(number(&result, "exit_code"), 0, "{result:?}");
            assert!(!tools.process_completion_waiting(), "wait {wait}");
            assert!(observer(&tools).completions_since(0).1.is_empty());
        }
    }
}

#[test]
fn a_requested_stop_reaches_the_model_without_asking_it_to_run() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let observer = observer(&tools);
    let (id, _) = start(&tools, "sleep 20");
    observer.request_stop(&id).unwrap();
    wait_for(|| !observer.processes().unwrap()[0].active());
    assert!(!tools.process_completion_waiting());
    let completions = tools.process_completions(&Redactor::empty());
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].status, "stopped");
    assert!(completions[0].stop_requested);
}

#[test]
fn a_stop_requested_after_the_end_leaves_the_outcome_alone() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let observer = observer(&tools);
    let (id, _) = start(&tools, &format!("{}; exit 3", gate("go")));
    release(&directory, "go");
    wait_for(|| tools.process_completion_waiting());
    assert_eq!(observer.request_stop(&id).unwrap().exit_code, Some(3));
    // It still ended by itself, so it still gives the model a reason to run.
    assert!(tools.process_completion_waiting());
    assert!(!tools.process_completions(&Redactor::empty())[0].stop_requested);
}

#[test]
fn a_rejected_process_call_acts_on_nothing() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let (id, _) = start(&tools, &gate("go"));
    let arguments = Value::object([
        ("action", Value::string("write")),
        ("process_id", Value::string(&id)),
        ("text", Value::string("input")),
        ("wait_ms", Value::string("soon")),
    ]);
    // Rejected before the terminal check, which precedes delivering input.
    assert_eq!(
        text(&tools.execute("process", &arguments.encode()), "error"),
        "wait_ms must be a nonnegative integer"
    );
    release(&directory, "go");
}

#[test]
fn numbering_waits_for_a_reader_already_scanning_the_list() {
    let scope = Arc::new(Scope::default());
    let entry = Arc::new(Entry {
        terminal: None,
        id: "process-fixture".into(),
        command: "fixture".into(),
        streams: ["stdout", "stderr"].map(|name| Stream {
            reference: name.into(),
            path: name.into(),
        }),
        stop: Cancellation::default(),
        state: Mutex::new(State::default()),
        changed: Condvar::new(),
        monitor: Mutex::new(None),
        #[cfg(all(test, unix))]
        capture_probe: None,
        started: Instant::now(),
        scope: Arc::downgrade(&scope),
    });
    // A reader past this entry must not find a later number on another one.
    let scanning = scope.entries.lock().unwrap();
    let ending = Arc::clone(&entry);
    let finished = thread::spawn(move || {
        ending.transition(|state| state.outcome = Some(exited()));
    });
    thread::sleep(Duration::from_millis(100));
    assert_eq!(entry.state.lock().unwrap().sequence, None);
    drop(scanning);
    finished.join().unwrap();
    assert_eq!(entry.state.lock().unwrap().sequence, Some(1));
}

#[test]
fn a_report_that_cannot_read_its_output_still_owes_the_outcome() {
    let directory = Directory::new();
    let scope = Arc::new(Scope::default());
    let entry = Entry {
        terminal: None,
        id: "process-fixture".into(),
        command: "fixture".into(),
        streams: ["stdout", "stderr"].map(|name| Stream {
            reference: name.into(),
            path: directory.path().join(name),
        }),
        stop: Cancellation::default(),
        state: Mutex::new(State {
            outcome: Some(exited()),
            waiters: 1,
            ..State::default()
        }),
        changed: Condvar::new(),
        monitor: Mutex::new(None),
        #[cfg(all(test, unix))]
        capture_probe: None,
        started: Instant::now(),
        scope: Arc::downgrade(&scope),
    };
    assert!(
        entry
            .report(WaitEnd::Elapsed, DEFAULT_OUTPUT_LIMIT, OutputMode::Next)
            .is_err()
    );
    let state = entry.state.lock().unwrap();
    assert_eq!((state.sequence, state.reported), (Some(1), false));
}

fn exited() -> Outcome {
    Outcome {
        status: "exited",
        exit_code: Some(0),
        error: None,
        stdout_bytes: 0,
        stderr_bytes: 0,
        supervised: true,
    }
}

#[test]
fn every_completion_ending_together_reaches_a_reader_once() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let observer = observer(&tools);
    let ids: Vec<_> = (0..8).map(|_| start(&tools, &gate("go")).0).collect();
    release(&directory, "go");
    let mut seen = Vec::new();
    let mut cursor = 0;
    wait_for(|| {
        let (next, completions) = observer.completions_since(cursor);
        cursor = next;
        seen.extend(completions.into_iter().map(|done| done.id));
        seen.len() >= ids.len()
    });
    seen.sort();
    let mut expected = ids;
    expected.sort();
    assert_eq!(seen, expected);
}

#[test]
fn completions_mask_credentials_and_bound_their_text() {
    let directory = Directory::new();
    let redactor = Redactor::new("private-key-value".into());
    let tools = tools(&directory, redactor.clone());
    let long = "x".repeat(400);
    let command = format!(
        "printf 'private-key-value\\n'; printf '%3000s' y; {}; # {long}",
        gate("go")
    );
    let (_, _) = start(&tools, &command);
    release(&directory, "go");
    wait_for(|| tools.process_completion_waiting());
    let completions = tools.process_completions(&redactor);
    let completion = &completions[0];
    assert!(
        completion.tails[0].len() <= 1024,
        "{}",
        completion.tails[0].len()
    );
    assert!(completion.command.chars().count() <= 201);
    let observer = tools.observe_processes(redactor.clone());
    let saved = observer.read(&completion.id, 0, Window::After(0)).unwrap();
    assert!(!saved.text.contains("private-key-value"));
    assert!(!completion.report().contains("private-key-value"));

    let masked = {
        let (_, _) = start(
            &tools,
            &format!("{}; printf done; # private-key-value", gate("masked")),
        );
        release(&directory, "masked");
        wait_for(|| tools.process_completions(&redactor).len() == 2);
        tools.process_completions(&redactor)
    };
    assert!(masked[1].command.contains("[redacted]"), "{:?}", masked[1]);
    assert!(!masked[1].report().contains("private-key-value"));
}

#[test]
fn a_tail_is_masked_even_when_the_saved_stream_is_not() {
    let directory = Directory::new();
    // The spool saves raw bytes here, so only the completion can mask them.
    let tools = tools(&directory, Redactor::empty());
    let (_, _) = start(
        &tools,
        &format!("printf 'private-key-value\\n'; {}", gate("go")),
    );
    release(&directory, "go");
    wait_for(|| tools.process_completion_waiting());
    let completions = tools.process_completions(&Redactor::new("private-key-value".into()));
    assert!(!completions[0].tails[0].is_empty());
    assert!(!completions[0].tails[0].contains("private-key-value"));
    assert!(!completions[0].report().contains("private-key-value"));
}

#[test]
fn a_listed_command_is_bounded_after_masking() {
    let directory = Directory::new();
    let secret = "private-key-value";
    let tools = tools(&directory, Redactor::new(secret.into()));
    // The cut at 200 characters lands five into the credential; masking
    // first leaves none of it.
    let prefix = format!("{}; # ", gate("go"));
    let command = format!("{prefix}{}{secret}", "x".repeat(195 - prefix.len()));
    let (_, _) = start(&tools, &command);
    let listed = tools.execute("process", r#"{"action":"list"}"#);
    let shown = text(
        &listed.get("processes").unwrap().as_array().unwrap()[0],
        "command",
    );
    assert_eq!(shown.chars().count(), 201, "{shown}");
    assert!(shown.ends_with('…'), "{shown}");
    assert!(!shown.contains("priva"), "{shown}");
    release(&directory, "go");
}

#[test]
fn old_finished_processes_leave_the_list_once_the_model_has_seen_them() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let first = bash(&tools, "printf first", &[]);
    assert_eq!(number(&first, "exit_code"), 0);
    for _ in 0..40 {
        bash(&tools, "true", &[]);
    }
    let processes = observer(&tools).processes().unwrap();
    assert!(processes.len() <= 33, "{}", processes.len());
    assert!(processes.iter().all(|process| process.command == "true"));
    // A saved reference outlives its place in the list.
    assert_eq!(
        std::fs::read_to_string(tools.outputs.resolve(&text(&first, "stdout_file")).unwrap())
            .unwrap(),
        "first"
    );
}

#[test]
fn an_unreported_completion_is_never_forgotten_to_make_room() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let (id, _) = start(&tools, &gate("go"));
    release(&directory, "go");
    wait_for(|| tools.process_completion_waiting());
    for _ in 0..40 {
        bash(&tools, "true", &[]);
    }
    let completions = tools.process_completions(&Redactor::empty());
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].id, id);
}

#[test]
fn ending_the_conversation_stops_processes_and_their_events() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let observer = observer(&tools);
    let woken = Arc::new(AtomicBool::new(false));
    let listener = Arc::clone(&woken);
    observer.on_change(Arc::new(move || listener.store(true, Ordering::SeqCst)));
    let (id, _) = start(&tools, HEARTBEAT);
    assert!(
        woken.swap(false, Ordering::SeqCst),
        "a launch wakes the interface"
    );
    wait_for(|| directory.path().join("heartbeat").exists());
    tools.stop_processes();
    assert!(heartbeat_stopped(&directory));
    assert!(observer.processes().is_none());
    assert_eq!(observer.background_count(), 0);
    assert!(observer.completions_since(0).1.is_empty());
    assert!(!tools.process_completion_waiting());
    assert!(tools.process_completions(&Redactor::empty()).is_empty());
    assert!(process(&tools, "poll", &id, None).get("error").is_some());
}

#[test]
fn a_finished_process_wakes_the_interface() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let observer = observer(&tools);
    let (_, _) = start(&tools, &gate("go"));
    let woken = Arc::new(AtomicBool::new(false));
    let listener = Arc::clone(&woken);
    observer.on_change(Arc::new(move || listener.store(true, Ordering::SeqCst)));
    release(&directory, "go");
    wait_for(|| woken.load(Ordering::SeqCst));
    wait_for(|| !only(&observer).active());
}
