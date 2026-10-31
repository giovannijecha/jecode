use super::*;
use crate::test_support::{Directory, HttpFixture, tool_call};

fn large_result_can_fit_by_preview(field: &str, forced: bool) {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = Agent::new(
        crate::backend::openrouter::OpenRouter::fixture(fixture.endpoint.clone()),
        crate::tools::Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent
        .prepare_turn("Inspect once and report; preserve original files.")
        .unwrap();
    let original_output = format!("HEAD Ω 日本\n{}\nTAIL", "x".repeat(80000));
    agent.messages.lock().unwrap().extend([
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(
                    "once",
                    "bash",
                    Value::object([("command", Value::string("inspect-once"))]),
                )]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("once")),
            (
                "content",
                Value::string(
                    Value::object([
                        ("exit_code", Value::number(0)),
                        (
                            "content",
                            Value::string(if field == "content" {
                                &original_output
                            } else {
                                ""
                            }),
                        ),
                        (
                            "stdout",
                            Value::string(if field == "stdout" {
                                &original_output
                            } else {
                                ""
                            }),
                        ),
                        ("stderr", Value::string("")),
                    ])
                    .encode(),
                ),
            ),
        ]),
    ]);
    let original = agent.messages.lock().unwrap().clone();
    let old_summary = agent.context.summary.clone();
    agent.context.observe(
        Some(&crate::backend::Usage {
            input_tokens: Some(6000),
            ..Default::default()
        }),
        2,
    );
    agent.context.calibrate(36000);
    let limits = Some(Limits {
        context: 24000,
        output: None,
    });
    assert_eq!(
        agent
            .compact_if_needed(limits, forced, &mut |event| {
                if matches!(event, Event::Recovering { .. }) {
                    return Err("A model summary was not needed".into());
                }
                Ok(())
            })
            .unwrap(),
        forced
    );
    assert_eq!(agent.context.from, 1);
    assert_eq!(agent.context.summary, old_summary);
    assert_eq!(*agent.messages.lock().unwrap(), original);
    let projected = agent.transport_messages();
    assert!(context::bytes(&projected) < 16000);
    assert!(
        projected
            .iter()
            .any(|message| message.encode().contains("history:3")
                && message.encode().contains("context_truncated"))
    );
    assert!(
        projected
            .iter()
            .any(|message| message.get("tool_calls").is_some())
    );
    let handle = agent.sessions().unwrap();
    let saved = handle.store().fixture_load(&handle.id()).unwrap();
    assert_eq!(saved.messages, original);
    assert_eq!(saved.context.preview_until, 4);
    assert!(
        saved.events.iter().any(|event| {
            event.get("command").and_then(Value::as_str) == Some("Context previews")
        })
    );
    let retrieved = agent.execute_tool(&crate::backend::openrouter::ToolCall {
        id: "original".into(),
        name: "read".into(),
        arguments: Value::object([("path", Value::string("history:3"))]).encode(),
    });
    assert!(
        retrieved
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("HEAD")
    );
    assert!(fixture.finish().is_empty());
}

#[test]
fn a_large_stdout_result_uses_a_preview_without_a_model_summary() {
    large_result_can_fit_by_preview("stdout", false);
}

#[test]
fn a_large_read_content_uses_a_preview_without_a_model_summary() {
    large_result_can_fit_by_preview("content", false);
}

#[test]
fn a_forced_context_recovery_tries_a_preview_before_a_model_summary() {
    large_result_can_fit_by_preview("content", true);
}

#[test]
fn a_provider_context_rejection_retries_with_a_preview_and_keeps_the_tool_pair() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            400,
            Value::object([(
                "error",
                Value::object([
                    ("code", Value::string("context_length_exceeded")),
                    ("message", Value::string("Maximum context length exceeded")),
                ]),
            )]),
        ),
        (
            200,
            crate::test_support::completion("Inspection complete.", vec![]),
        ),
    ]);
    let mut agent = Agent::new(
        crate::backend::openrouter::OpenRouter::fixture(fixture.endpoint.clone()),
        crate::tools::Tools::new(directory.path()).unwrap(),
    );
    agent.set_auto_compact(99);
    agent.client.fixture_limits(160_000, Some(4096));
    agent.prepare_turn("Inspect once").unwrap();
    let original_output = format!("HEAD\n{}\nTAIL", "evidence ".repeat(10_000));
    agent.messages.lock().unwrap().extend([
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(
                    "once",
                    "read",
                    Value::object([("path", Value::string("source"))]),
                )]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("once")),
            (
                "content",
                Value::string(
                    Value::object([("content", Value::string(&original_output))]).encode(),
                ),
            ),
        ]),
    ]);
    let answer = agent
        .next_completion(
            Some(Limits {
                context: 160_000,
                output: Some(4096),
            }),
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(answer.text, "Inspection complete.");
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].body.get("tools").is_some());
    assert!(requests[1].body.get("tools").is_some());
    assert!(requests[1].body.encode().contains("context_truncated"));
    assert!(requests[1].body.encode().contains("history:3"));
    assert!(requests[1].body.encode().contains("tool_calls"));
    assert_eq!(agent.context.from, 1);
    let messages = agent.messages.lock().unwrap();
    let saved = crate::json::parse(messages[3].get("content").unwrap().as_str().unwrap()).unwrap();
    assert_eq!(
        saved.get("content").and_then(Value::as_str),
        Some(original_output.as_str())
    );
}
