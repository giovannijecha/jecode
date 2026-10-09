use super::super::*;
use crate::openrouter::Limits;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};

fn input(agent: &mut Agent) -> (Vec<Value>, Vec<(usize, String)>, String, String) {
    let mut messages = agent.messages.lock().unwrap();
    messages.push(Value::object([
        ("role", Value::string("user")),
        (
            "content",
            Value::string("Inspect source; keep manual changes."),
        ),
    ]));
    messages.push(Value::object([
        ("role", Value::string("assistant")),
        (
            "tool_calls",
            Value::Array(vec![tool_call(
                "read",
                "read",
                Value::object([("path", Value::string("source.rs"))]),
            )]),
        ),
    ]));
    messages.push(Value::object([
        ("role", Value::string("tool")),
        ("tool_call_id", Value::string("read")),
        (
            "content",
            Value::string(
                Value::object([("content", Value::string("Source payload Ω ".repeat(800)))])
                    .encode(),
            ),
        ),
    ]));
    messages.push(Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string("Last record.")),
    ]));
    let original = messages.clone();
    let records = messages
        .iter()
        .enumerate()
        .skip(2)
        .map(|(index, message)| (index, message.encode()))
        .collect();
    drop(messages);
    agent.context.from = 2;
    let mut valid = crate::json::parse(&crate::context::memory::fixture("Inspect source")).unwrap();
    if let Value::Object(fields) = &mut valid {
        fields.insert(
            "remaining".into(),
            Value::Array(vec![Value::string(
                "Preserve detailed pending facts. ".repeat(100),
            )]),
        );
    }
    let accepted = valid.encode();
    if let Value::Object(fields) = &mut valid {
        fields.insert(
            "completed".into(),
            Value::Array(vec![Value::object([
                ("description", Value::string("Check passed")),
                ("kind", Value::string("check")),
                ("evidence", Value::Array(vec![Value::string("history:3")])),
            ])]),
        );
    }
    (original, records, accepted, valid.encode())
}

#[test]
fn native_proof_repair_keeps_the_rejected_portion_boundary_without_resending_payloads() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let (original, records, accepted, rejected) = input(&mut agent);
    let fixture = HttpFixture::new(vec![
        (200, completion(&rejected, vec![])),
        (200, completion(&accepted, vec![])),
    ]);
    agent.replace_client(OpenRouter::fixture(fixture.endpoint.clone()));
    let result = agent
        .summarize(
            &records,
            "Inspect source; keep manual changes.",
            Limits {
                context: 24000,
                output: None,
            },
            8192,
            &mut |event| {
                if matches!(event, Event::Recovering { .. }) {
                    return Err("Unexpected extra summary request".into());
                }
                Ok(())
            },
        )
        .unwrap()
        .0;
    assert!(result.contains("Preserve detailed pending facts."));
    assert_eq!(*agent.messages.lock().unwrap(), original);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    let body = |at: usize| {
        requests[at]
            .body
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()[1]
            .get("content")
            .unwrap()
            .as_str()
            .unwrap()
    };
    assert!(body(0).contains("Last record."));
    assert!(body(1).contains("Native sources for the rejected transcript portion"));
    assert!(body(1).contains("history:4"));
    assert!(body(1).contains("source.rs"));
    assert!(!body(1).contains("Source payload"));
    assert!(requests[1].body.encode().len() < requests[0].body.encode().len());
}

#[test]
fn a_provider_context_rejection_falls_back_without_discarding_original_evidence() {
    let directory = Directory::new();
    let empty = HttpFixture::new(vec![]);
    let mut agent = Agent::new(
        OpenRouter::fixture(empty.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let (original, records, accepted, rejected) = input(&mut agent);
    let fixture = HttpFixture::new(vec![
        (200, completion(&rejected, vec![])),
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
        (200, completion(&accepted, vec![])),
    ]);
    agent.replace_client(OpenRouter::fixture(fixture.endpoint.clone()));
    agent
        .summarize(
            &records[..1],
            "Inspect source",
            Limits {
                context: 24000,
                output: None,
            },
            8192,
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(*agent.messages.lock().unwrap(), original);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert!(
        requests[1]
            .body
            .encode()
            .contains("Native sources for the rejected transcript portion")
    );
    assert!(
        requests[2]
            .body
            .encode()
            .contains("Original message, byte 0")
    );
    assert!(requests[2].body.encode().contains("tool_calls"));
}

#[test]
fn repair_truncation_grows_allowance_and_retains_the_same_portion() {
    let directory = Directory::new();
    let empty = HttpFixture::new(vec![]);
    let mut agent = Agent::new(
        OpenRouter::fixture(empty.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let (original, records, accepted, rejected) = input(&mut agent);
    let truncated = Value::object([(
        "choices",
        Value::Array(vec![Value::object([
            ("finish_reason", Value::string("length")),
            (
                "message",
                Value::object([
                    ("role", Value::string("assistant")),
                    ("content", Value::string("{")),
                ]),
            ),
        ])]),
    )]);
    let fixture = HttpFixture::new(vec![
        (200, completion(&rejected, vec![])),
        (200, truncated),
        (200, completion(&accepted, vec![])),
    ]);
    agent.replace_client(OpenRouter::fixture(fixture.endpoint.clone()));
    let (_, allowance) = agent
        .summarize(
            &records,
            "Inspect source",
            Limits {
                context: 24000,
                output: None,
            },
            8192,
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(allowance, 6000);
    assert_eq!(*agent.messages.lock().unwrap(), original);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    for request in &requests[1..] {
        assert!(
            request
                .body
                .encode()
                .contains("Native sources for the rejected transcript portion")
        );
        assert!(request.body.encode().contains("history:4"));
    }
}
