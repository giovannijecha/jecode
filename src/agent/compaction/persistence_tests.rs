use super::*;
use crate::test_support::{Directory, HttpFixture, completion};

#[test]
fn failed_compaction_checkpoint_cannot_be_saved_later() {
    check_failed_checkpoint(false);
}

#[test]
fn failed_preview_checkpoint_cannot_be_saved_later() {
    check_failed_checkpoint(true);
}

fn check_failed_checkpoint(preview: bool) {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(if preview {
        vec![]
    } else {
        vec![(200, completion("Earlier findings preserved.", vec![]))]
    });
    let mut agent = Agent::new(
        crate::backend::openrouter::OpenRouter::fixture(fixture.endpoint.clone()),
        crate::tools::Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent
        .prepare_turn("Inspect; preserve protected.txt.")
        .unwrap();
    let message = |role: &str, content: &str| {
        Value::object([
            ("role", Value::string(role)),
            ("content", Value::string(content)),
        ])
    };
    let limits = if preview {
        agent.messages.lock().unwrap().extend([
            Value::object([
                ("role", Value::string("assistant")),
                (
                    "tool_calls",
                    Value::Array(vec![crate::test_support::tool_call(
                        "large",
                        "bash",
                        Value::object([("command", Value::string("inspect"))]),
                    )]),
                ),
            ]),
            Value::object([
                ("role", Value::string("tool")),
                ("tool_call_id", Value::string("large")),
                (
                    "content",
                    Value::string(
                        Value::object([("stdout", Value::string("x".repeat(80_000)))]).encode(),
                    ),
                ),
            ]),
        ]);
        agent.context.observe(
            Some(&crate::backend::Usage {
                input_tokens: Some(6000),
                ..Default::default()
            }),
            2,
        );
        agent.context.calibrate(36_000);
        Limits {
            context: 24_000,
            output: None,
        }
    } else {
        agent.messages.lock().unwrap().extend([
            message(
                "assistant",
                &"Earlier findings ".repeat(crate::tools::encoded_size() / 8),
            ),
            message("user", "Check the dates next."),
        ]);
        let fixed = agent.current_system_message().encode().len()
            + crate::tools::encoded_size()
            + agent.request_environment().len();
        Limits {
            context: (fixed + 8192).div_ceil(4) * 5,
            output: Some(4096),
        }
    };
    agent.save_session().unwrap();
    let handle = agent.sessions().unwrap();
    let previous = agent.context.value();
    let journal = handle
        .store()
        .bucket()
        .join(format!("{}.jsonl", handle.id()));
    let original_permissions = std::fs::metadata(&journal).unwrap().permissions();
    let mut readonly = original_permissions.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&journal, readonly).unwrap();
    let result = agent.compact_if_needed(Some(limits), !preview, &mut |event| {
        assert!(!matches!(event, Event::ContextCompacted { .. }));
        Ok(())
    });
    std::fs::set_permissions(&journal, original_permissions).unwrap();
    let error = result.unwrap_err();
    assert!(error.contains("read-only"), "{error}");
    assert_eq!(agent.context.value(), previous);
    assert_eq!(handle.snapshot().context.value(), previous);
    assert!(!agent.events.lock().unwrap().iter().any(is_compaction));
    agent.save_session().unwrap();
    let saved = handle.store().fixture_load(&handle.id()).unwrap();
    assert_eq!(saved.context.value(), previous);
    assert!(!saved.events.iter().any(is_compaction));
    assert_eq!(fixture.finish().len(), usize::from(!preview));
}

fn is_compaction(event: &Value) -> bool {
    matches!(
        event.get("command").and_then(Value::as_str),
        Some("Context compaction" | "Context previews")
    )
}
