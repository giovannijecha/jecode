use super::*;
use crate::test_support::{Directory, HttpFixture, tool_call};

#[test]
fn a_large_result_can_fit_by_preview_without_a_model_summary_or_losing_original_proof() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = Agent::new(
        crate::openrouter::OpenRouter::fixture(fixture.endpoint.clone()),
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
                        ("stdout", Value::string(&original_output)),
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
        Some(&Value::object([("prompt_tokens", Value::number(6000))])),
        2,
    );
    agent.context.calibrate(36000);
    let limits = Some(Limits {
        context: 24000,
        output: None,
    });
    assert!(
        !agent
            .compact_if_needed(limits, false, &mut |event| {
                if matches!(event, Event::Recovering { .. }) {
                    return Err("A model summary was not needed".into());
                }
                Ok(())
            })
            .unwrap()
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
    let retrieved = agent.execute_tool(&crate::openrouter::ToolCall {
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
