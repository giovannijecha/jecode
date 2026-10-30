use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    cancel::Cancellation,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::{
    fs,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[test]
fn started_agent_call_captures_reused_number_without_changing_model_arguments() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("First report", vec![])),
        (200, completion("Second report", vec![])),
        (200, completion("Third report", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let create = Value::object([
        ("action", Value::string("create")),
        (
            "tasks",
            Value::Array(vec![Value::object([
                ("title", Value::string("Inspect")),
                ("instructions", Value::string("Return a report")),
                ("workspace", Value::string("shared")),
            ])]),
        ),
    ]);
    for id in ["a1", "a2"] {
        agent.agent_tool(&create).unwrap();
        agent
            .collaboration()
            .wait(Some(id), Duration::from_secs(5), &Cancellation::default())
            .unwrap();
        agent.collaboration().close(id).unwrap();
    }
    agent.agent_tool(&create).unwrap();
    let raw = Value::object([
        ("action", Value::string("wait")),
        ("id", Value::string("a3")),
        ("display_number", Value::number(99)),
    ]);
    let call = ToolCall {
        id: "wait".into(),
        name: "agent".into(),
        arguments: raw.encode(),
    };
    let Event::ToolStarted { arguments, .. } = agent.started_event(&call) else {
        panic!("Expected agent tool start");
    };
    assert_eq!(arguments.get("display_number"), Some(&Value::number(1)));
    assert_eq!(arguments.get("id").and_then(Value::as_str), Some("a3"));
    assert_eq!(call.arguments, raw.encode());
    agent
        .collaboration()
        .wait(Some("a3"), Duration::from_secs(5), &Cancellation::default())
        .unwrap();
    agent.collaboration().stop_all();
    assert_eq!(fixture.finish().len(), 3);
}
fn read_call(id: &str, path: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "read".into(),
        arguments: Value::object([("path", Value::string(path))]).encode(),
    }
}

#[test]
fn history_and_skill_reads_stay_serial() {
    assert!(parallel_call(&read_call("file", "src/main.rs")));
    assert!(!parallel_call(&read_call("history", "history:requests")));
    assert!(!parallel_call(&read_call("skill", "skill:example")));
    assert!(!parallel_call(&ToolCall {
        name: "bash".into(),
        ..read_call("shell", "src/main.rs")
    }));
}

#[test]
fn parallel_operations_exclude_state_changes_and_shared_cursors() {
    let call = |name: &str, arguments: Value| ToolCall {
        id: "fixture".into(),
        name: name.into(),
        arguments: arguments.encode(),
    };
    for (name, action) in [
        ("map", "overview"),
        ("checkpoint", "list"),
        ("checkpoint", "inspect"),
        ("process", "list"),
        ("web", "fetch"),
        ("web", "download"),
    ] {
        assert!(parallel_call(&call(
            name,
            Value::object([("action", Value::string(action))])
        )));
    }
    for (name, action) in [
        ("map", "remember"),
        ("map", "forget"),
        ("checkpoint", "restore"),
        ("process", "poll"),
        ("process", "stop"),
        ("process", "write"),
        ("web", "crawl"),
        ("browser", "snapshot"),
    ] {
        assert!(!parallel_call(&call(
            name,
            Value::object([("action", Value::string(action))])
        )));
    }
    for arguments in [
        Value::object([
            ("action", Value::string("fetch")),
            ("method", Value::string("POST")),
        ]),
        Value::object([
            ("action", Value::string("fetch")),
            ("body", Value::string("payload")),
        ]),
    ] {
        assert!(!parallel_call(&call("web", arguments)));
    }
}

#[test]
fn independent_reads_overlap_and_results_keep_source_order() {
    let started = Arc::new((Mutex::new(0usize), Condvar::new()));
    let calls = [read_call("first", "a"), read_call("second", "b")];
    let results = run_parallel_calls(&calls, Clock::new(), &|call| {
        let (count, changed) = &*started;
        let mut count = count.lock().unwrap();
        *count += 1;
        changed.notify_all();
        let (count, timeout) = changed
            .wait_timeout_while(count, Duration::from_secs(2), |count| *count < 2)
            .unwrap();
        assert!(!timeout.timed_out() && *count == 2);
        Execution {
            result: Value::object([("id", Value::string(&call.id))]),
            change: None,
        }
    });
    assert_eq!(
        results[0].0.result.get("id").and_then(Value::as_str),
        Some("first")
    );
    assert_eq!(
        results[1].0.result.get("id").and_then(Value::as_str),
        Some("second")
    );
}

#[test]
fn cancellation_waits_for_every_started_read() {
    let started = Arc::new((Mutex::new(0usize), Condvar::new()));
    let finished = AtomicUsize::new(0);
    let cancellation = Cancellation::default();
    let calls = [read_call("first", "a"), read_call("second", "b")];
    let results = run_parallel_calls(&calls, Clock::new(), &|call| {
        let (count, changed) = &*started;
        let mut count = count.lock().unwrap();
        *count += 1;
        changed.notify_all();
        let (count, timeout) = changed
            .wait_timeout_while(count, Duration::from_secs(2), |count| *count < 2)
            .unwrap();
        assert!(!timeout.timed_out() && *count == 2);
        drop(count);
        if call.id == "first" {
            cancellation.cancel();
        } else {
            while !cancellation.requested() {
                std::thread::yield_now();
            }
        }
        finished.fetch_add(1, Ordering::SeqCst);
        Execution {
            result: Value::object([("cancelled", Value::Bool(cancellation.requested()))]),
            change: None,
        }
    });
    assert_eq!(finished.load(Ordering::SeqCst), 2);
    assert!(
        results
            .iter()
            .all(|(result, _)| result.result.get("cancelled") == Some(&Value::Bool(true)))
    );
}

#[test]
fn read_batch_stops_at_a_write_and_next_read_sees_the_change() {
    let directory = Directory::new();
    let home = Directory::new();
    fs::write(directory.path().join("a.txt"), "old").unwrap();
    fs::write(directory.path().join("b.txt"), "other").unwrap();
    let calls = vec![
        tool_call(
            "a_before",
            "read",
            Value::object([("path", Value::string("a.txt"))]),
        ),
        tool_call(
            "b_before",
            "read",
            Value::object([("path", Value::string("b.txt"))]),
        ),
        tool_call(
            "write",
            "write",
            Value::object([
                ("path", Value::string("a.txt")),
                ("content", Value::string("new")),
            ]),
        ),
        tool_call(
            "a_after",
            "read",
            Value::object([("path", Value::string("a.txt"))]),
        ),
    ];
    let fixture = HttpFixture::new(vec![
        (200, completion("", calls)),
        (200, completion("Done.", vec![])),
    ]);
    let mut agent = Agent::new(
        crate::backend::openrouter::OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent
        .run_turn("Read, write, read.", &mut |_| Ok(()))
        .unwrap();
    let messages = agent.messages.lock().unwrap();
    let ids: Vec<_> = messages[3..7]
        .iter()
        .map(|message| message.get("tool_call_id").and_then(Value::as_str).unwrap())
        .collect();
    assert_eq!(ids, ["a_before", "b_before", "write", "a_after"]);
    assert!(
        messages[3]
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("old")
    );
    assert!(
        messages[6]
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("new")
    );
    drop(messages);
    let saved = agent.sessions().unwrap().snapshot();
    assert_eq!(saved.messages[3..7].len(), 4);
    assert!(!saved.pending.active);
    assert!(saved.pending.reads.is_empty());
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn cancelled_read_batch_records_results_and_does_not_start_later_write() {
    let directory = Directory::new();
    fs::write(directory.path().join("a.txt"), "a").unwrap();
    fs::write(directory.path().join("b.txt"), "b").unwrap();
    let calls = vec![
        tool_call(
            "a",
            "read",
            Value::object([("path", Value::string("a.txt"))]),
        ),
        tool_call(
            "b",
            "read",
            Value::object([("path", Value::string("b.txt"))]),
        ),
        tool_call(
            "later",
            "write",
            Value::object([
                ("path", Value::string("must-not-exist")),
                ("content", Value::string("no")),
            ]),
        ),
    ];
    let fixture = HttpFixture::new(vec![(200, completion("", calls))]);
    let mut agent = Agent::new(
        crate::backend::openrouter::OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let cancellation = agent.cancellation();
    let error = agent
        .run_turn("Read both, then write.", &mut |event| {
            if matches!(event, Event::ToolStarted { id, .. } if id == "b") {
                cancellation.cancel();
            }
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error, "Operation cancelled");
    assert!(!directory.path().join("must-not-exist").exists());
    let messages = agent.messages.lock().unwrap();
    for (message, id) in messages[3..6].iter().zip(["a", "b", "later"]) {
        assert_eq!(
            message.get("tool_call_id").and_then(Value::as_str),
            Some(id)
        );
    }
    assert!(
        messages[5]
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("not executed")
    );
    drop(messages);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn failed_start_event_cancels_unstarted_read_batch_and_later_write() {
    let directory = Directory::new();
    let calls = vec![
        tool_call(
            "a",
            "read",
            Value::object([("path", Value::string("a.txt"))]),
        ),
        tool_call(
            "b",
            "read",
            Value::object([("path", Value::string("b.txt"))]),
        ),
        tool_call(
            "later",
            "write",
            Value::object([
                ("path", Value::string("must-not-exist")),
                ("content", Value::string("no")),
            ]),
        ),
    ];
    let fixture = HttpFixture::new(vec![(200, completion("", calls))]);
    let mut agent = Agent::new(
        crate::backend::openrouter::OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let error = agent
        .run_turn("Read first.", &mut |event| {
            if matches!(event, Event::ToolStarted { .. }) {
                Err("display failed".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error, "display failed");
    assert!(!directory.path().join("must-not-exist").exists());
    let messages = agent.messages.lock().unwrap();
    assert!(messages[3..6].iter().all(|message| {
        message
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("not executed")
    }));
    drop(messages);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn failed_finish_event_keeps_all_read_results_and_cancels_later_write() {
    let directory = Directory::new();
    let home = Directory::new();
    fs::write(directory.path().join("a.txt"), "alpha").unwrap();
    fs::write(directory.path().join("b.txt"), "beta").unwrap();
    let calls = vec![
        tool_call(
            "a",
            "read",
            Value::object([("path", Value::string("a.txt"))]),
        ),
        tool_call(
            "b",
            "read",
            Value::object([("path", Value::string("b.txt"))]),
        ),
        tool_call(
            "later",
            "write",
            Value::object([
                ("path", Value::string("must-not-exist")),
                ("content", Value::string("no")),
            ]),
        ),
    ];
    let fixture = HttpFixture::new(vec![(200, completion("", calls))]);
    let mut agent = Agent::new(
        crate::backend::openrouter::OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let error = agent
        .run_turn("Read first.", &mut |event| {
            if matches!(event, Event::ToolFinished { .. }) {
                Err("display failed".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error, "display failed");
    assert!(!directory.path().join("must-not-exist").exists());
    let saved = agent.sessions().unwrap().snapshot();
    assert!(!saved.pending.active);
    assert!(
        saved.messages[3]
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("alpha")
    );
    assert!(
        saved.messages[4]
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("beta")
    );
    assert!(
        saved.messages[5]
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("not executed")
    );
    assert_eq!(fixture.finish().len(), 1);
}

#[cfg(windows)]
#[test]
fn failed_batch_result_checkpoint_recovers_uncertain_reads_without_replaying_write() {
    let directory = Directory::new();
    let home = Directory::new();
    fs::write(directory.path().join("a.txt"), "alpha").unwrap();
    fs::write(directory.path().join("b.txt"), "beta").unwrap();
    let calls = vec![
        tool_call(
            "a",
            "read",
            Value::object([("path", Value::string("a.txt"))]),
        ),
        tool_call(
            "b",
            "read",
            Value::object([("path", Value::string("b.txt"))]),
        ),
        tool_call(
            "later",
            "write",
            Value::object([
                ("path", Value::string("must-not-exist")),
                ("content", Value::string("no")),
            ]),
        ),
    ];
    let fixture = HttpFixture::new(vec![(200, completion("", calls))]);
    let mut agent = Agent::new(
        crate::backend::openrouter::OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent.prepare_turn("Read first.").unwrap();
    let handle = agent.sessions().unwrap();
    let id = handle.id();
    let store = handle.store();
    let bucket = fs::read_dir(home.path().join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let journal = bucket.join(format!("{id}.jsonl"));
    let original = fs::metadata(&journal).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    let error = agent.run_turn("Read first.", &mut |event| {
        if matches!(event, Event::ToolStarted { id, .. } if id == "b") {
            fs::set_permissions(&journal, readonly.clone()).unwrap();
        }
        Ok(())
    });
    fs::set_permissions(&journal, original).unwrap();
    assert!(error.is_err());
    assert!(!directory.path().join("must-not-exist").exists());
    drop(agent);
    drop(handle);
    let (recovered, _) = crate::sessions::Handle::open(
        store,
        &id,
        crate::redact::Redactor::new("fixture-key".into()),
    )
    .unwrap();
    let saved = recovered.snapshot();
    assert!(!saved.pending.active);
    for (message, outcome) in
        saved.messages[3..6]
            .iter()
            .zip(["unknown", "unknown", "not_executed"])
    {
        let value = json::parse(message.get("content").and_then(Value::as_str).unwrap()).unwrap();
        assert_eq!(value.get("outcome").and_then(Value::as_str), Some(outcome));
    }
    assert_eq!(fixture.finish().len(), 1);
}
