use super::*;
use crate::test_support::{Directory, HttpFixture, completion};

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
                    "original record ".repeat(800)
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
fn chronological_portions_include_every_record_and_carry_the_previous_summary() {
    for calibrated in [false, true] {
        let directory = Directory::new();
        let candidate = String::from("Continue the original task.");
        let expected = if calibrated { 1 } else { 2 };
        let fixture = HttpFixture::new(vec![(200, completion(&candidate, vec![])); expected]);
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
                crate::backend::openrouter::Limits {
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
        assert_eq!(requests.len(), expected);
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
                request_bytes.div_ceil(2)
            } <= 21000
        );
        let supplied_text = supplied(request);
        assert!(supplied_text.contains(&previous));
        for (_, record) in &records {
            assert!(
                requests
                    .iter()
                    .any(|request| supplied(request).contains(record))
            );
        }
        if !calibrated {
            assert!(supplied(&requests[1]).contains(&candidate));
        }
        assert!(
            requests
                .iter()
                .all(|request| !request.body.encode().contains("Older messages are omitted"))
        );
    }
}

#[test]
fn a_rejected_summary_retries_a_smaller_portion_without_skipping_older_records() {
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
            crate::backend::openrouter::Limits {
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
    assert!(supplied(&requests[1]).contains(&records[0].1));
    for (_, record) in &records {
        assert!(
            requests[1..]
                .iter()
                .any(|request| supplied(request).contains(record))
        );
    }
    assert!(supplied(&requests[2]).contains(&candidate));
}
