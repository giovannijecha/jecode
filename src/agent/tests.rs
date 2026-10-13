use super::*;
use crate::console::Console;
use crate::events::Event;
use crate::json;
use crate::test_support::{Directory, HttpFixture, completion as response, tool_call as call};
use std::fs;

#[test]
fn failed_message_display_keeps_the_provider_message_and_cancels_its_tools() {
    let directory = Directory::new();
    let first = response(
        "Received but not displayed.",
        vec![call(
            "pending",
            "write",
            Value::object([
                ("path", Value::string("must-not-exist")),
                ("content", Value::string("pending")),
            ]),
        )],
    );
    let fixture = HttpFixture::new(vec![
        (200, first.clone()),
        (200, response("Recovered.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let error = agent
        .run_turn("Write the fixture.", &mut |event| {
            if matches!(event, Event::Message { .. }) {
                Err("fixture display failure".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error, "fixture display failure");
    assert!(!directory.path().join("must-not-exist").exists());
    let document = agent.archive().document();
    let messages = document.get("messages").unwrap().as_array().unwrap();
    assert_eq!(
        &messages[2],
        first.get("choices").unwrap().as_array().unwrap()[0]
            .get("message")
            .unwrap()
    );
    assert_eq!(
        messages[3].get("tool_call_id").and_then(Value::as_str),
        Some("pending")
    );
    agent.run_turn("Continue.", &mut |_| Ok(())).unwrap();
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn completes_all_four_tools_and_keeps_context_for_the_next_turn() {
    let directory = Directory::new();
    fs::write(directory.path().join("source.txt"), "before\n").unwrap();
    let first = response(
        "I will inspect the file.",
        vec![call(
            "read_1",
            "read",
            Value::object([("path", Value::string("source.txt"))]),
        )],
    );
    let fixture = HttpFixture::new(vec![
        (200, first.clone()),
        (
            200,
            response(
                "",
                vec![
                    call(
                        "write_1",
                        "write",
                        Value::object([
                            ("path", Value::string("new.txt")),
                            ("content", Value::string("new file\n")),
                        ]),
                    ),
                    call(
                        "edit_1",
                        "edit",
                        Value::object([
                            ("path", Value::string("source.txt")),
                            ("old_text", Value::string("before")),
                            ("new_text", Value::string("after")),
                        ]),
                    ),
                    call(
                        "bash_1",
                        "bash",
                        Value::object([("command", Value::string("cat source.txt"))]),
                    ),
                ],
            ),
        ),
        (200, response("Updated and verified.", vec![])),
        (
            200,
            response("The previous change is still in context.", vec![]),
        ),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let mut output = Vec::new();
    let mut events = Vec::new();
    agent
        .run_turn(
            "Inspect, create, edit and verify.",
            &mut Console::new(&mut output, &mut events),
        )
        .unwrap();
    assert_eq!(
        fs::read_to_string(directory.path().join("source.txt")).unwrap(),
        "after\n"
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("new.txt")).unwrap(),
        "new file\n"
    );
    agent
        .run_turn(
            "What did you change?",
            &mut Console::new(&mut output, &mut events),
        )
        .unwrap();
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("Updated and verified.")
    );
    let events = String::from_utf8(events).unwrap();
    assert_eq!(
        events.matches("[waiting for model: fixture/model]").count(),
        2
    );
    for name in ["read", "write", "edit", "bash"] {
        assert!(events.contains(&format!("[tool: {name}]")));
    }
    assert!(events.contains("[bash: exit 0]"));
    let requests = fixture.finish();
    let second = requests[1]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    let assistant = first.get("choices").unwrap().as_array().unwrap()[0]
        .get("message")
        .unwrap();
    assert_eq!(&second[2], assistant);
    assert_eq!(
        second[3].get("tool_call_id").unwrap().as_str(),
        Some("read_1")
    );
    let third = requests[2]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    let result = json::parse(third[7].get("content").unwrap().as_str().unwrap()).unwrap();
    assert_eq!(result.get("stdout").unwrap().as_str(), Some("after\n"));
    assert_eq!(result.get("exit_code").unwrap().as_usize(), Some(0));
    for request in &requests {
        assert_eq!(
            request.body.get("tools").unwrap().as_array().unwrap().len(),
            5
        );
    }
    let fourth = requests[3]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        fourth.last().unwrap().get("content").unwrap().as_str(),
        Some("What did you change?")
    );
    assert!(
        fourth
            .iter()
            .any(|message| message.get("tool_call_id").and_then(Value::as_str) == Some("edit_1"))
    );
    agent.clear();
    assert_eq!(agent.messages.lock().unwrap().len(), 1);
}

#[test]
fn sends_tool_errors_back_so_the_model_can_repair_its_call() {
    let directory = Directory::new();
    let malformed = Value::object([
        ("id", Value::string("bad")),
        ("type", Value::string("function")),
        (
            "function",
            Value::object([
                ("name", Value::string("write")),
                ("arguments", Value::string("{bad")),
            ]),
        ),
    ]);
    let fixture = HttpFixture::new(vec![
        (200, response("", vec![malformed])),
        (
            200,
            response(
                "",
                vec![call(
                    "fixed",
                    "write",
                    Value::object([
                        ("path", Value::string("fixed.txt")),
                        ("content", Value::string("fixed")),
                    ]),
                )],
            ),
        ),
        (200, response("Corrected.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.run_turn("Create a file.", &mut |_| Ok(())).unwrap();
    assert_eq!(
        fs::read_to_string(directory.path().join("fixed.txt")).unwrap(),
        "fixed"
    );
    let requests = fixture.finish();
    let messages = requests[1]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    assert!(
        messages
            .last()
            .unwrap()
            .get("content")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("Invalid JSON")
    );
}

#[test]
fn a_long_turn_executes_tools_beyond_the_previous_request_limit() {
    let directory = Directory::new();
    fs::write(directory.path().join("source"), "fixture").unwrap();
    let requests = 80;
    let mut responses: Vec<_> = (0..requests - 2)
        .map(|index| {
            (
                200,
                response(
                    "",
                    vec![call(
                        &format!("call_{index}"),
                        "read",
                        Value::object([("path", Value::string("source"))]),
                    )],
                ),
            )
        })
        .collect();
    responses.push((
        200,
        response(
            "",
            vec![call(
                "last",
                "write",
                Value::object([
                    ("path", Value::string("completed")),
                    ("content", Value::string("x")),
                ]),
            )],
        ),
    ));
    responses.push((200, response("All done.", vec![])));
    let fixture = HttpFixture::new(responses);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent
        .run_turn("Complete a long task.", &mut |_| Ok(()))
        .unwrap();
    assert_eq!(
        fs::read_to_string(directory.path().join("completed")).unwrap(),
        "x"
    );
    assert_eq!(
        agent
            .messages
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .get("role")
            .unwrap()
            .as_str(),
        Some("assistant")
    );
    assert_eq!(fixture.finish().len(), requests);
}

#[test]
fn failed_tool_event_output_cancels_calls_without_corrupting_history() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            response(
                "",
                vec![call(
                    "cancelled",
                    "write",
                    Value::object([
                        ("path", Value::string("must-not-exist")),
                        ("content", Value::string("x")),
                    ]),
                )],
            ),
        ),
        (200, response("Recovered.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    assert!(
        agent
            .run_turn("Create a file.", &mut |event| match event {
                Event::ToolStarted { .. } => Err("fixture output failure".into()),
                _ => Ok(()),
            })
            .is_err()
    );
    assert!(!directory.path().join("must-not-exist").exists());
    agent.run_turn("Continue.", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    let messages = requests[1]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(messages.len(), 5);
    assert_eq!(
        messages[3].get("tool_call_id").and_then(Value::as_str),
        Some("cancelled")
    );
    assert!(
        messages[3]
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("not executed")
    );
}

#[test]
fn interrupted_result_display_keeps_completed_work_and_cancels_remaining_calls() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            response(
                "",
                vec![
                    call(
                        "completed",
                        "write",
                        Value::object([
                            ("path", Value::string("completed.txt")),
                            ("content", Value::string("done")),
                        ]),
                    ),
                    call(
                        "pending",
                        "write",
                        Value::object([
                            ("path", Value::string("must-not-exist")),
                            ("content", Value::string("pending")),
                        ]),
                    ),
                ],
            ),
        ),
        (200, response("Recovered.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let mut observed = Vec::new();
    let error = agent
        .run_turn("Write both files.", &mut |event| {
            let failed = matches!(event, Event::ToolFinished { .. });
            observed.push(event);
            if failed {
                Err("fixture display failure".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error, "fixture display failure");
    assert_eq!(
        fs::read_to_string(directory.path().join("completed.txt")).unwrap(),
        "done"
    );
    assert!(!directory.path().join("must-not-exist").exists());
    let completed_result = json::parse(
        agent.messages.lock().unwrap()[3]
            .get("content")
            .and_then(Value::as_str)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        observed,
        vec![
            Event::Waiting {
                model: "fixture/model".into()
            },
            Event::ToolStarted {
                id: "completed".into(),
                name: "write".into(),
                arguments: Value::object([
                    ("path", Value::string("completed.txt")),
                    ("content", Value::string("done"))
                ]),
            },
            Event::ToolFinished {
                id: "completed".into(),
                name: "write".into(),
                summary: "wrote 4 bytes".into(),
                result: completed_result,
            },
        ]
    );
    agent.run_turn("Continue.", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    let messages = requests[1]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(
        messages[3].get("tool_call_id").and_then(Value::as_str),
        Some("completed")
    );
    assert_eq!(
        messages[4].get("tool_call_id").and_then(Value::as_str),
        Some("pending")
    );
    assert!(
        messages[3]
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("bytes_written")
    );
    assert!(
        messages[4]
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("not executed")
    );
}
