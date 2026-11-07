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
            let id = text(&result, "process_id");
            let observer = observer(&tools);
            wait_for(|| !only(&observer).active());
            wait_for(|| {
                tools.process_completion_waiting()
                    || text(&process(&tools, "poll", &id, None), "status") == "exited"
            });
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
        let (_, _) = start(&tools, "printf done; # private-key-value");
        wait_for(|| tools.process_completions(&redactor).len() == 2);
        tools.process_completions(&redactor)
    };
    assert!(masked[1].command.contains("[redacted]"), "{:?}", masked[1]);
    assert!(!masked[1].report().contains("private-key-value"));
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
