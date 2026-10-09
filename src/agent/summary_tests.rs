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
    agent.context.summary = Value::object([
        ("objective", Value::string("Continue the original task.")),
        ("constraints", Value::Array(vec![Value::string(constraint)])),
        (
            "completed",
            Value::Array(vec![Value::object([
                (
                    "description",
                    Value::string("Preservation requirement recorded."),
                ),
                ("kind", Value::string("decision")),
                ("evidence", Value::Array(vec![Value::string("history:1")])),
            ])]),
        ),
        (
            "remaining",
            Value::Array(vec![Value::string("Verify original behavior.")]),
        ),
        ("next_action", Value::string("Verify original behavior.")),
    ])
    .encode();
    (original, records)
}

#[test]
fn calibrated_summary_fits_more_original_records_without_losing_carried_work() {
    for (calibrated, count) in [(false, 2), (true, 1)] {
        let directory = Directory::new();
        let candidate = crate::context::memory::fixture("Continue the original task.");
        let fixture = HttpFixture::new(vec![(200, completion(&candidate, vec![])); count]);
        let mut agent = Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        );
        let (original, records) = summary_input(&mut agent, calibrated);
        let previous = crate::json::parse(&agent.context.summary).unwrap();
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
        let current = crate::json::parse(&summary).unwrap();
        for field in ["constraints", "completed"] {
            assert_eq!(current.get(field), previous.get(field));
        }
        assert!(summary.contains("Verify original behavior."));
        assert_eq!(*agent.messages.lock().unwrap(), original);
        let requests = fixture.finish();
        assert_eq!(requests.len(), count);
        let mut supplied = String::new();
        for request in requests {
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
            supplied.push_str(
                request.body.get("messages").unwrap().as_array().unwrap()[1]
                    .get("content")
                    .unwrap()
                    .as_str()
                    .unwrap(),
            );
        }
        for (index, record) in records {
            assert!(supplied.contains(&format!("history:{index}")));
            assert!(supplied.contains(&record));
        }
    }
}

#[test]
fn rejected_calibrated_summary_retries_smaller_portions_without_skipping_records() {
    let directory = Directory::new();
    let candidate = crate::context::memory::fixture("Continue the original task.");
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
    assert_eq!(requests.len(), 3);
    assert!(requests[1].body.encode().len() < requests[0].body.encode().len());
    let supplied = requests[1..]
        .iter()
        .map(|request| {
            request.body.get("messages").unwrap().as_array().unwrap()[1]
                .get("content")
                .unwrap()
                .as_str()
                .unwrap()
        })
        .collect::<String>();
    for (_, record) in records {
        assert!(supplied.contains(&record));
    }
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
fn invalid_memory_is_repaired_and_the_large_latest_tool_group_stays_usable() {
    let directory = Directory::new();
    let original = "Original evidence Ω 日本\n".repeat(1900);
    std::fs::write(directory.path().join("source"), &original).unwrap();
    let fixture = HttpFixture::new(vec![
        (200, large_read()),
        (
            200,
            completion("Implementation complete; finish checks.", vec![]),
        ),
        (
            200,
            completion(
                &crate::context::memory::fixture("Inspect source; preserve protected.txt."),
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
    assert_eq!(agent.context.from, 2);
    let requests = fixture.finish();
    assert!(requests[1].body.get("tools").is_none());
    assert!(
        requests[2]
            .body
            .encode()
            .contains("Native continuity validation result")
    );
    let messages = requests[3]
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
            .iter()
            .any(|message| message.encode().contains("context_truncated")
                && message.encode().contains("history:3"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.get("tool_calls").is_some())
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
fn repeated_invalid_memory_preserves_the_previous_context_and_saved_evidence() {
    let directory = Directory::new();
    let home = Directory::new();
    std::fs::write(
        directory.path().join("source"),
        "Original evidence\n".repeat(1900),
    )
    .unwrap();
    let fixture = HttpFixture::new(vec![
        (200, large_read()),
        (200, completion("Done.", vec![])),
        (200, completion("Still done.", vec![])),
    ]);
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    client.fixture_limits(24000, Some(4096));
    let mut agent = Agent::new(client, Tools::new(directory.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    let error = agent
        .run_turn("Inspect source", &mut |_| Ok(()))
        .unwrap_err();
    assert!(error.contains("could not be validated"));
    assert_eq!(agent.context.from, 1);
    assert!(agent.context.summary.is_empty());
    let handle = agent.sessions().unwrap();
    let saved = handle.store().fixture_load(&handle.id()).unwrap();
    assert_eq!(saved.context.from, 1);
    assert!(saved.messages[3].encode().contains("Original evidence"));
    assert_eq!(fixture.finish().len(), 3);
}

#[test]
fn a_later_portion_failure_records_the_actual_intermediate_memory_for_replay() {
    let directory = Directory::new();
    let first = crate::context::memory::fixture("Current goal from the first portion");
    let mut rejected = crate::json::parse(&first).unwrap();
    if let Value::Object(fields) = &mut rejected {
        fields.insert(
            "completed".into(),
            Value::Array(vec![Value::string("Not proof")]),
        );
    }
    let rejected = rejected.encode();
    let fixture = HttpFixture::new(vec![
        (200, completion(&first, vec![])),
        (200, completion(&rejected, vec![])),
        (200, completion(&rejected, vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let (original, records) = summary_input(&mut agent, false);
    let context = agent.context.value();
    let error = agent
        .summarize(
            &records,
            "Preserve original tests",
            crate::openrouter::Limits {
                context: 24000,
                output: None,
            },
            4096,
            &mut |_| Ok(()),
        )
        .unwrap_err();
    assert!(error.contains("completed[0] must be an object"), "{error}");
    assert_eq!(agent.context.value(), context);
    assert_eq!(*agent.messages.lock().unwrap(), original);
    let events = agent.events.lock().unwrap();
    let failures = events
        .iter()
        .filter(|event| {
            event.get("command").and_then(Value::as_str) == Some("Context memory validation")
        })
        .collect::<Vec<_>>();
    assert_eq!(failures.len(), 2);
    for failure in failures {
        let detail = |key: &str| {
            failure
                .get("details")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .find_map(|pair| {
                    let pair = pair.as_array()?;
                    (pair[0].as_str() == Some(key)).then(|| pair[1].as_str().unwrap())
                })
                .unwrap()
        };
        let previous = detail("previous_memory");
        assert!(previous.contains("Current goal from the first portion"));
        assert!(previous.contains("Verify original behavior."));
        assert!(previous.contains("history:1"));
        let replay = crate::context::memory::prepare_with_notices(
            detail("rejected_memory"),
            previous,
            &original,
            detail("evidence_since").parse().unwrap(),
            detail("memory_limit").parse().unwrap(),
        )
        .unwrap_err();
        assert_eq!(
            Some(replay.as_str()),
            failure.get("result").and_then(Value::as_str)
        );
    }
    assert_eq!(fixture.finish().len(), 3);
}

#[test]
fn carried_constraints_can_grow_beyond_a_quarter_of_the_context() {
    let directory = Directory::new();
    std::fs::write(
        directory.path().join("source"),
        "Original evidence\n".repeat(1900),
    )
    .unwrap();
    let mut state =
        crate::json::parse(&crate::context::memory::fixture("Continue existing work")).unwrap();
    if let Value::Object(fields) = &mut state {
        fields.insert(
            "constraints".into(),
            Value::Array(vec![Value::string(
                "Preserve detailed constraint; ".repeat(210),
            )]),
        );
    }
    let memory = state.encode();
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
