use super::*;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};
use std::fs;

fn check(id: &str, marker: &str) -> Value {
    tool_call(
        id,
        "bash",
        Value::object([
            (
                "command",
                Value::string(format!("printf '{marker}\\n' >> executions.txt")),
            ),
            ("check", Value::Bool(true)),
        ]),
    )
}

fn ledger() -> String {
    Value::object([
        ("objective", Value::string("Complete the same ongoing work")),
        (
            "constraints",
            Value::Array(vec![Value::string("Keep original file constraints")]),
        ),
        (
            "remaining",
            Value::Array(vec![Value::string("Run the new outstanding check")]),
        ),
        (
            "next_action",
            Value::string("Run the new outstanding check once"),
        ),
        (
            "completed",
            Value::Array(
                (0..250)
                    .map(|index| {
                        Value::object([
                            (
                                "description",
                                Value::string(format!(
                                    "Retained inspection {index}: {}",
                                    "Exact earlier evidence. ".repeat(10)
                                )),
                            ),
                            ("kind", Value::string("inspection")),
                            ("evidence", Value::Array(vec![Value::string("history:3")])),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
    .encode()
}

#[test]
fn a_large_completed_ledger_compacts_in_a_small_window_and_remains_readable_after_resume() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut reviewed = crate::json::parse(&crate::context::memory::fixture(
        "Continue the current work",
    ))
    .unwrap();
    if let Value::Object(fields) = &mut reviewed {
        fields.insert(
            "constraints".into(),
            Value::Array(vec![Value::string("Keep original file constraints")]),
        );
        fields.insert(
            "remaining".into(),
            Value::Array(vec![Value::string("Run the new outstanding check")]),
        );
    }
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![check("first", "first")])),
        (200, completion("Original check performed", vec![])),
        (200, completion(&reviewed.encode(), vec![])),
        (200, completion("", vec![check("second", "second")])),
        (200, completion("New check performed", vec![])),
    ]);
    let mut first = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    first.enable_sessions(home.path()).unwrap();
    first
        .run_turn("Perform the first check.", &mut |_| Ok(()))
        .unwrap();
    first.context.summary = ledger();
    first.context.from = first.messages.lock().unwrap().len();
    first.context.reset_usage();
    first.client.fixture_limits(24000, Some(4096));
    let mut compactions = 0;
    first
        .run_turn("Continue with the outstanding check once.", &mut |event| {
            if matches!(event, Event::ContextCompacted { .. }) {
                compactions += 1;
            }
            if matches!(event, Event::Recovering { .. }) {
                return Err("Unexpected recovery in the fixed memory fixture".into());
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(compactions, 1);
    assert!(first.context.summary.len() > 50000);
    assert!(first.context.memory_view().len() <= first.context.memory_limit.unwrap());
    assert_eq!(
        fs::read_to_string(directory.path().join("executions.txt")).unwrap(),
        "first\nsecond\n"
    );
    let id = first.sessions().unwrap().id();
    let durable = first.context.summary.clone();
    drop(first);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 5);
    assert!(requests[2].body.get("tools").is_none());
    assert!(requests[3].body.encode().contains("history:memory"));

    let mut resumed = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/unused".into()),
        Tools::new(directory.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.context.summary, durable);
    let mut text = String::new();
    let mut offset = 0;
    loop {
        let page = resumed.execute_tool(&ToolCall {
            id: "read-memory".into(),
            name: "read".into(),
            arguments: Value::object([
                ("path", Value::string("history:memory")),
                ("byte_offset", Value::number(offset)),
            ])
            .encode(),
        });
        assert!(page.get("error").is_none(), "{}", page.encode());
        text.push_str(page.get("content").and_then(Value::as_str).unwrap());
        match page.get("next_byte_offset").and_then(Value::as_usize) {
            Some(next) => {
                assert!(next > offset);
                offset = next;
            }
            None => break,
        }
    }
    let memory = crate::json::parse(&text).unwrap();
    assert_eq!(
        memory.get("completed").unwrap().as_array().unwrap().len(),
        250
    );
    assert_eq!(
        memory.get("constraints").unwrap().as_array().unwrap(),
        &[Value::string("Keep original file constraints")]
    );
    assert!(
        memory
            .get("remaining")
            .unwrap()
            .encode()
            .contains("outstanding")
    );
}
