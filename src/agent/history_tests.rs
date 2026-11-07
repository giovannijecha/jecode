use super::*;
use crate::test_support::{Directory, HttpFixture, completion};

fn read(agent: &Agent, path: &str, byte_offset: usize) -> Value {
    agent.execute_tool(&ToolCall {
        id: "history-read".into(),
        name: "read".into(),
        arguments: Value::object([
            ("path", Value::string(path)),
            ("byte_offset", Value::number(byte_offset)),
        ])
        .encode(),
    })
}

#[test]
fn a_saved_process_outcome_is_readable_after_resume_without_restarting_it() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Verify the earlier build")),
    ]));
    agent.save_session().unwrap();
    let event = Value::object([
        ("type", Value::string("process_finished")),
        ("process_id", Value::string("past-process")),
        ("command", Value::string("verify\x1b[2J")),
        ("status", Value::string("exited")),
        ("exit_code", Value::number(7)),
        ("stdout_file", Value::string("saved-output")),
    ]);
    agent
        .sessions()
        .unwrap()
        .append_event(&agent.archive(), event)
        .unwrap();
    let id = agent.sessions().unwrap().id();
    drop(agent);
    let mut resumed = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    let page = read(&resumed, "history:processes", 0);
    assert!(page.get("error").is_none(), "{}", page.encode());
    let content = page.get("content").and_then(Value::as_str).unwrap();
    assert!(content.contains("past-process"), "{content}");
    assert!(content.contains(r#""exit_code":7"#), "{content}");
    assert!(content.contains("saved-output"), "{content}");
    assert!(resumed.processes().processes().unwrap().is_empty());
    assert!(!resumed.can_continue_processes());
    let saved = resumed.sessions().unwrap().snapshot();
    let event = saved
        .events
        .iter()
        .find(|event| event.get("process_id").and_then(Value::as_str) == Some("past-process"))
        .unwrap();
    let line = crate::console::restored_process_line(event);
    assert!(line.contains("exit 7"), "{line}");
    assert!(!line.contains('\x1b'), "{line}");
    assert!(fixture.finish().is_empty());
}

#[test]
fn returned_input_is_not_readable_through_numeric_or_request_history() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(
        404,
        Value::object([(
            "error",
            Value::object([("message", Value::string("No endpoint"))]),
        )]),
    )]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let input = agent.input_checkpoint();
    assert!(
        agent
            .run_turn("withdrawn-fixture-input", &mut |_| Ok(()))
            .is_err()
    );
    assert!(agent.return_input(input));
    let page = read(&agent, "history:1", 0);
    assert!(
        page.get("error")
            .and_then(Value::as_str)
            .is_some_and(|error| error.contains("withdrawn")),
        "{}",
        page.encode()
    );
    assert!(!page.encode().contains("withdrawn-fixture-input"));
    assert!(
        !read(&agent, "history:requests", 0)
            .encode()
            .contains("withdrawn-fixture-input")
    );
    // Withdrawal changes the model's view, not the diagnostic source archive.
    assert!(
        agent
            .archive()
            .document()
            .encode()
            .contains("withdrawn-fixture-input")
    );
    fixture.finish();
}

#[test]
fn large_original_requests_are_fully_readable_after_compaction_and_resume() {
    let directory = Directory::new();
    let home = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let original = "Preserve Ω and 日本; full-range arithmetic.\r\n".repeat(2400);
    agent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("user")),
        ("content", Value::string(&original)),
    ]));
    agent.context.from = 2;
    agent.context.summary = "Legacy memory; continue verification".into();
    agent.save_session().unwrap();
    let id = agent.sessions().unwrap().id();
    drop(agent);
    let mut resumed = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    let mut offset = 0;
    let mut restored = String::new();
    loop {
        let page = read(&resumed, "history:1", offset);
        assert!(page.get("error").is_none(), "{}", page.encode());
        restored.push_str(page.get("content").and_then(Value::as_str).unwrap());
        if let Some(next) = page.get("next_byte_offset").and_then(Value::as_usize) {
            assert!(next > offset);
            offset = next;
        } else {
            break;
        }
    }
    assert_eq!(restored, original);
    let view = resumed.transport_messages();
    assert!(
        view.iter()
            .any(|message| message.get("role").and_then(Value::as_str) == Some("user"))
    );
    assert!(
        view.iter()
            .any(|message| message.encode().contains("history:1"))
    );
}

#[test]
fn history_is_read_only_and_bad_references_never_fall_back_to_filesystem_access() {
    let directory = Directory::new();
    let agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    for path in [
        "history:../outside",
        "history:-1",
        "history:999",
        "history:",
    ] {
        assert!(read(&agent, path, 0).get("error").is_some());
    }
    for name in ["write", "edit"] {
        let call = ToolCall {
            id: "deny".into(),
            name: name.into(),
            arguments: Value::object([
                ("path", Value::string("history:0")),
                ("content", Value::string("change")),
                ("old_text", Value::string("Agent")),
                ("new_text", Value::string("change")),
            ])
            .encode(),
        };
        assert!(agent.execute_tool(&call).get("error").is_some());
    }
    assert_eq!(agent.messages.lock().unwrap().len(), 1);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn explicit_history_reads_preserve_native_metadata_and_mask_configured_credentials() {
    let directory = Directory::new();
    let agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string("isolated-fixture-key")),
        ("reasoning_details", Value::string("private-provider-state")),
    ]));
    let page = read(&agent, "history:1", 0).encode();
    assert!(!page.contains("isolated-fixture-key"));
    assert!(page.contains("private-provider-state"));
    assert!(page.contains("[redacted]"));
    agent.messages.lock().unwrap().extend([
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![crate::test_support::tool_call(
                    "masked",
                    "isolated-fixture-key",
                    Value::object([]),
                )]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("masked")),
            ("content", Value::string("{}")),
        ]),
    ]);
    let page = read(&agent, "history:3", 0);
    assert_eq!(
        page.get("source").unwrap().get("tool"),
        Some(&Value::string("[redacted]"))
    );
    assert!(!page.encode().contains("isolated-fixture-key"));
}

#[test]
fn a_write_receipt_links_to_the_original_request_with_exact_file_contents() {
    let directory = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    let original = "first\r\nsecond without final newline";
    let arguments = Value::object([
        ("path", Value::string("source")),
        ("content", Value::string(original)),
    ]);
    agent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("assistant")),
        (
            "tool_calls",
            Value::Array(vec![crate::test_support::tool_call(
                "write",
                "write",
                arguments.clone(),
            )]),
        ),
    ]));
    let call = ToolCall {
        id: "write".into(),
        name: "write".into(),
        arguments: arguments.encode(),
    };
    let written = agent.execute_tool(&call);
    agent.tool_result(&call, &written);
    agent.context.from = 3;
    agent.context.summary = "Continue from the saved version.".into();
    let receipt = read(&agent, "history:2", 0);
    assert_eq!(
        receipt
            .get("request_history_reference")
            .and_then(Value::as_str),
        Some("history:1")
    );
    let request = read(&agent, "history:1", 0);
    let message =
        crate::json::parse(request.get("content").and_then(Value::as_str).unwrap()).unwrap();
    let arguments = message.get("tool_calls").unwrap().as_array().unwrap()[0]
        .get("function")
        .unwrap()
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap();
    assert_eq!(
        crate::json::parse(arguments)
            .unwrap()
            .get("content")
            .and_then(Value::as_str),
        Some(original)
    );
}

#[test]
fn paged_history_exposes_the_executed_action_owner_after_compaction_and_a_new_request() {
    let directory = Directory::new();
    let arguments = Value::object([
        ("path", Value::string("draft.txt")),
        ("content", Value::string("Draft\r\n")),
    ]);
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![crate::test_support::tool_call("draft", "write", arguments)],
            ),
        ),
        (200, completion("Draft delivered", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.run_turn("Build a draft", &mut |_| Ok(())).unwrap();
    agent
        .prepare_turn("Cancel the draft task; audit only")
        .unwrap();
    agent.context.from = 5;
    agent.context.summary = "Old draft task".into();
    let original = agent.messages.lock().unwrap().clone();
    for offset in [0, 8] {
        let page = read(&agent, "history:3", offset);
        let source = page.get("source").unwrap();
        assert_eq!(source.get("role"), Some(&Value::string("tool")));
        assert_eq!(
            source.get("request_history"),
            Some(&Value::string("history:1"))
        );
        assert_eq!(
            source.get("call_history"),
            Some(&Value::string("history:2"))
        );
        assert_eq!(source.get("tool"), Some(&Value::string("write")));
        // Retrieving an earlier write result is not itself a new write.
        assert!(page.get("bytes_written").is_none());
    }
    assert_eq!(*agent.messages.lock().unwrap(), original);
    assert_eq!(
        std::fs::read(directory.path().join("draft.txt")).unwrap(),
        b"Draft\r\n"
    );
    assert_eq!(fixture.finish().len(), 2);
}
