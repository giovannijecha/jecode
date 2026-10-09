use super::*;
use crate::test_support::{Directory, HttpFixture, completion};

fn setup(endpoint: &str, directory: &Directory) -> Agent {
    let mut agent = Agent::new(
        OpenRouter::fixture(endpoint.into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.messages.lock().unwrap().extend([
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Keep data.txt; build an index.")),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("Index unfinished")),
        ]),
        Value::object([
            ("role", Value::string("user")),
            (
                "content",
                Value::string("Cancel the index. Audit the report; keep data.txt."),
            ),
        ]),
    ]);
    agent.context.summary = Value::object([
        ("objective", Value::string("Build index")),
        (
            "constraints",
            Value::Array(vec![Value::string("Keep data.txt")]),
        ),
        ("completed", Value::Array(vec![])),
        (
            "remaining",
            Value::Array(vec![Value::string("Build index")]),
        ),
        ("next_action", Value::string("Build index")),
    ])
    .encode();
    agent
}

fn proposal(reviewed: bool) -> Value {
    let old = setup("http://127.0.0.1:1/chat/completions", &Directory::new())
        .context
        .summary;
    let labels = crate::json::parse(&crate::context::memory::prompt_view(&old)).unwrap();
    let label = |key| {
        labels.get(key).unwrap().as_array().unwrap()[0]
            .as_str()
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_string()
    };
    let mut candidate =
        crate::json::parse(&crate::context::memory::fixture("Audit report only")).unwrap();
    if reviewed && let Value::Object(fields) = &mut candidate {
        fields.insert(
            "constraints".into(),
            Value::Array(vec![Value::string(label("constraints"))]),
        );
        fields.insert(
            "resolved".into(),
            Value::Array(vec![Value::object([
                ("description", Value::string(label("remaining"))),
                ("kind", Value::string("decision")),
                ("evidence", Value::Array(vec![Value::string("history:3")])),
            ])]),
        );
    }
    candidate
}

fn summarize(agent: &Agent) -> Result<String, String> {
    let records = agent
        .messages
        .lock()
        .unwrap()
        .iter()
        .enumerate()
        .skip(1)
        .map(|(index, message)| (index, message.encode()))
        .collect::<Vec<_>>();
    agent
        .summarize(
            &records,
            "history:3 Cancel the index; audit the report and preserve data.txt",
            crate::openrouter::Limits {
                context: 24000,
                output: None,
            },
            4096,
            &mut |_| Ok(()),
        )
        .map(|(summary, _)| summary)
}

#[test]
fn a_changed_request_rejects_implicit_carry_and_repairs_with_original_user_evidence() {
    let directory = Directory::new();
    let mut rejected = proposal(false);
    if let Value::Object(fields) = &mut rejected {
        fields.insert(
            "completed".into(),
            Value::Array(vec![Value::object([
                ("description", Value::string("Claimed inspection")),
                ("kind", Value::string("inspection")),
                ("evidence", Value::Array(vec![Value::string("history:2")])),
            ])]),
        );
    }
    let fixture = HttpFixture::new(vec![
        (200, completion(&rejected.encode(), vec![])),
        (200, completion(&proposal(true).encode(), vec![])),
    ]);
    let agent = setup(&fixture.endpoint, &directory);
    let original = agent.messages.lock().unwrap().clone();
    let previous = agent.context.value();
    let result = summarize(&agent).unwrap();
    let ledger = crate::json::parse(&result).unwrap();
    assert!(
        !ledger
            .get("remaining")
            .unwrap()
            .encode()
            .contains("Build index")
    );
    assert!(
        ledger
            .get("constraints")
            .unwrap()
            .encode()
            .contains("Keep data.txt")
    );
    assert_eq!(
        ledger
            .get("reviewed_request_history")
            .and_then(Value::as_usize),
        Some(3)
    );
    assert_eq!(*agent.messages.lock().unwrap(), original);
    assert_eq!(agent.context.value(), previous);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    let system = requests[0]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .get("content")
        .and_then(Value::as_str)
        .unwrap();
    assert!(system.contains(crate::context::memory::RESPONSE_SCHEMA));
    assert!(
        requests[0]
            .body
            .encode()
            .contains("Native requirement review")
    );
    assert!(
        requests[1]
            .body
            .encode()
            .contains("requires explicit review of prior requirements")
    );
    assert!(requests[1].body.encode().contains("history:3"));
    assert!(
        requests[1]
            .body
            .encode()
            .contains("completed[0] references history:2 (assistant)")
    );
}

#[test]
fn two_unreviewed_proposals_preserve_the_previous_context_and_original_messages() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion(&proposal(false).encode(), vec![])),
        (200, completion(&proposal(false).encode(), vec![])),
    ]);
    let agent = setup(&fixture.endpoint, &directory);
    let original = agent.messages.lock().unwrap().clone();
    let previous = agent.context.value();
    let error = summarize(&agent).unwrap_err();
    assert!(error.contains("requires explicit review"), "{error}");
    assert_eq!(*agent.messages.lock().unwrap(), original);
    assert_eq!(agent.context.value(), previous);
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn a_preview_does_not_age_a_new_check_but_an_accepted_summary_boundary_does() {
    let directory = Directory::new();
    let previous = Value::object([
        ("objective", Value::string("Verify the report")),
        ("constraints", Value::Array(vec![])),
        ("completed", Value::Array(vec![])),
        (
            "remaining",
            Value::Array(vec![Value::string("Verify report")]),
        ),
        ("next_action", Value::string("Verify report")),
        ("reviewed_request_history", Value::number(1)),
    ]);
    let labeled =
        crate::json::parse(&crate::context::memory::prompt_view(&previous.encode())).unwrap();
    let label = labeled.get("remaining").unwrap().as_array().unwrap()[0]
        .as_str()
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();
    let candidate = Value::object([
        ("objective", Value::string("Report verified results")),
        ("constraints", Value::Array(vec![])),
        (
            "completed",
            Value::Array(vec![Value::object([
                ("description", Value::string(label)),
                ("kind", Value::string("check")),
                ("evidence", Value::Array(vec![Value::string("history:3")])),
            ])]),
        ),
        (
            "remaining",
            Value::Array(vec![Value::string("Deliver report")]),
        ),
        ("next_action", Value::string("Deliver report")),
    ]);
    let fixture = HttpFixture::new(vec![
        (200, completion(&candidate.encode(), vec![])),
        (200, completion(&candidate.encode(), vec![])),
    ]);
    for from in [2, 4] {
        let mut agent = Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        );
        agent.messages.lock().unwrap().extend([
            Value::object([
                ("role", Value::string("user")),
                (
                    "content",
                    Value::string("Verify the report and deliver the result"),
                ),
            ]),
            Value::object([
                ("role", Value::string("assistant")),
                (
                    "tool_calls",
                    Value::Array(vec![crate::test_support::tool_call(
                        "verify",
                        "bash",
                        Value::object([
                            ("command", Value::string("verify-report")),
                            ("check", Value::Bool(true)),
                        ]),
                    )]),
                ),
            ]),
            Value::object([
                ("role", Value::string("tool")),
                ("tool_call_id", Value::string("verify")),
                (
                    "content",
                    Value::string(
                        Value::object([
                            ("exit_code", Value::number(0)),
                            ("check", Value::Bool(true)),
                            ("check_status", Value::string("passed")),
                        ])
                        .encode(),
                    ),
                ),
            ]),
        ]);
        agent.context.summary = previous.encode();
        agent.context.from = from;
        agent.context.preview_until = 4;
        let original = agent.messages.lock().unwrap().clone();
        let result = summarize(&agent).unwrap();
        let ledger = crate::json::parse(&result).unwrap();
        let remaining = ledger.get("remaining").unwrap().as_array().unwrap();
        assert_eq!(
            remaining.contains(&Value::string("Verify report")),
            from == 4
        );
        assert_eq!(*agent.messages.lock().unwrap(), original);
        assert_eq!(agent.context.summary, previous.encode());
    }
    assert_eq!(fixture.finish().len(), 2);
}
