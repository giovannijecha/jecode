use super::*;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};

fn summary_input(agent: &mut Agent, calibrated: bool) -> (Vec<Value>, Vec<(usize, String)>) {
    let constraint = "Preserve every original test byte.";
    let mut messages = agent.messages.lock().unwrap();
    messages.push(Value::object([
        ("role", Value::string("user")),
        ("content", Value::string(constraint)),
    ]));
    for index in 0..4 {
        messages.push(Value::object([
            ("role", Value::string("assistant")),
            (
                "content",
                Value::string(format!(
                    "{} end-{index}-Ω-日本",
                    "original record ".repeat(500)
                )),
            ),
        ]));
    }
    let original = messages.clone();
    let records = messages
        .iter()
        .enumerate()
        .skip(2)
        .map(|(index, message)| (index, message.encode()))
        .collect();
    drop(messages);
    agent.context.from = 2;
    if calibrated {
        agent.context.calibration = Some((6000, 36000));
    }
    agent.context.summary = format!(
        "Continue the original task. Constraint: {constraint} Next: verify original behavior (history:1)."
    );
    (original, records)
}

fn supplied(request: &crate::test_support::Request) -> &str {
    request.body.get("messages").unwrap().as_array().unwrap()[1]
        .get("content")
        .unwrap()
        .as_str()
        .unwrap()
}

#[test]
fn one_summary_request_keeps_the_newest_records_that_fit_and_the_previous_summary() {
    for calibrated in [false, true] {
        let directory = Directory::new();
        let candidate = String::from("Continue the original task.");
        let fixture = HttpFixture::new(vec![(200, completion(&candidate, vec![]))]);
        let mut agent = Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        );
        let (original, records) = summary_input(&mut agent, calibrated);
        let previous = agent.context.summary.clone();
        let summary = agent
            .summarize(
                &records,
                "Preserve every original test byte.",
                crate::openrouter::Limits {
                    context: 24000,
                    output: None,
                },
                4096,
                &mut |event| {
                    if matches!(event, Event::Recovering { .. }) {
                        return Err("Unexpected fixture request".into());
                    }
                    Ok(())
                },
            )
            .unwrap()
            .0;
        assert_eq!(summary, candidate);
        assert_eq!(*agent.messages.lock().unwrap(), original);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert!(request.body.get("tools").is_none());
        assert_eq!(
            request.body.get("max_completion_tokens"),
            Some(&Value::number(3000))
        );
        let messages = request.body.get("messages").unwrap().as_array().unwrap();
        let request_bytes = crate::context::bytes(messages).saturating_add(768);
        // At the measured 1:6 density, leave twice the observed input cost.
        assert!(
            if calibrated {
                request_bytes.div_ceil(3)
            } else {
                request_bytes
            } <= 21000
        );
        let supplied = supplied(request);
        assert!(supplied.contains(&previous));
        let (newest, _) = records.last().unwrap();
        assert!(supplied.contains(&format!("history:{newest}")));
        // Calibration fits every record; without it the oldest are left out.
        assert_eq!(
            records.iter().all(|(_, record)| supplied.contains(record)),
            calibrated
        );
        assert_eq!(supplied.contains("Older messages are omitted"), !calibrated);
    }
}

#[test]
fn a_rejected_summary_request_retries_with_fewer_old_records() {
    let directory = Directory::new();
    let candidate = String::from("Continue the original task.");
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
        (200, completion(&candidate, vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let (original, records) = summary_input(&mut agent, true);
    agent
        .summarize(
            &records,
            "Preserve every original test byte.",
            crate::openrouter::Limits {
                context: 24000,
                output: None,
            },
            4096,
            &mut |event| {
                if matches!(event, Event::Recovering { .. }) {
                    return Err("Unexpected fixture request".into());
                }
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(*agent.messages.lock().unwrap(), original);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].body.encode().len() < requests[0].body.encode().len());
    let retried = supplied(&requests[1]);
    assert!(retried.contains(&records.last().unwrap().1));
    assert!(retried.contains("Older messages are omitted"));
}

fn large_read() -> Value {
    let mut response = completion(
        "",
        vec![tool_call(
            "large",
            "read",
            Value::object([
                ("path", Value::string("source")),
                ("limit", Value::number(2000)),
            ]),
        )],
    );
    if let Value::Object(fields) = &mut response {
        fields.insert(
            "usage".into(),
            Value::object([("prompt_tokens", Value::number(6000))]),
        );
    }
    response
}

#[test]
fn a_large_latest_tool_result_is_summarized_and_stays_readable() {
    let directory = Directory::new();
    let original = "Original evidence Ω 日本\n".repeat(1900);
    std::fs::write(directory.path().join("source"), &original).unwrap();
    let fixture = HttpFixture::new(vec![
        (200, large_read()),
        (
            200,
            completion(
                &String::from("Inspect source; preserve protected.txt."),
                vec![],
            ),
        ),
        (200, completion("Verified inspection complete.", vec![])),
    ]);
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    client.fixture_limits(24000, Some(4096));
    let mut agent = Agent::new(client, Tools::new(directory.path()).unwrap());
    agent
        .run_turn("Inspect source; preserve protected.txt", &mut |event| {
            if matches!(event, Event::Recovering { .. }) {
                return Err("Unexpected fixture request".into());
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(agent.context.from, 4);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    let summary_request = requests[1].body.encode();
    assert!(requests[1].body.get("tools").is_none());
    // The summary sees a preview of the large result and its history reference.
    assert!(summary_request.contains("context_truncated"));
    assert!(summary_request.contains("history:3"));
    let messages = requests[2]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    assert!(
        messages.iter().any(
            |message| message.get("role").and_then(Value::as_str) == Some("user")
                && message.encode().contains("protected.txt")
        )
    );
    assert!(
        messages
            .last()
            .unwrap()
            .encode()
            .contains("Inspect source; preserve protected.txt.")
    );
    assert!(
        messages
            .iter()
            .all(|message| message.get("tool_calls").is_none())
    );
    let result = agent.execute_tool(&ToolCall {
        id: "retrieve".into(),
        name: "read".into(),
        arguments: Value::object([
            ("path", Value::string("history:3")),
            ("byte_offset", Value::number(0)),
        ])
        .encode(),
    });
    assert!(
        result
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("Original evidence")
    );
    assert!(agent.messages.lock().unwrap()[3].encode().len() > 50000);
}

#[test]
fn carried_constraints_can_grow_beyond_a_quarter_of_the_context() {
    let directory = Directory::new();
    std::fs::write(
        directory.path().join("source"),
        "Original evidence\n".repeat(1900),
    )
    .unwrap();
    let memory = format!(
        "Continue existing work. Constraints: {}end.",
        "Preserve detailed constraint; ".repeat(210)
    );
    assert!(memory.len() > 5250 && memory.len() < 8192);
    let fixture = HttpFixture::new(vec![
        (200, large_read()),
        (200, completion(&memory, vec![])),
        (200, completion("Finished.", vec![])),
    ]);
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    client.fixture_limits(24000, Some(4096));
    let mut agent = Agent::new(client, Tools::new(directory.path()).unwrap());
    agent.prepare_turn("Continue existing work").unwrap();
    agent.context.from = 2;
    agent.context.summary = memory.clone();
    agent
        .run_turn("Continue existing work", &mut |event| {
            if matches!(event, Event::Recovering { .. }) {
                return Err("Unexpected fixture request".into());
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(agent.context.summary, memory);
    assert_eq!(fixture.finish().len(), 3);
}
