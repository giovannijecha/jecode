use super::super::*;
use crate::backend::openrouter::Limits;
use crate::test_support::{Directory, HttpFixture, completion};

fn candidate() -> String {
    "Inspect only".into()
}

fn truncated() -> Value {
    Value::object([(
        "choices",
        Value::Array(vec![Value::object([
            ("finish_reason", Value::string("length")),
            (
                "message",
                Value::object([
                    ("role", Value::string("assistant")),
                    ("content", Value::string("{\"objective\":")),
                ]),
            ),
        ])]),
    )])
}

fn historical_notes(agent: &Agent) {
    agent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("assistant")),
        (
            "content",
            Value::string("Earlier inspection notes. ".repeat(450)),
        ),
    ]));
}

#[test]
fn a_large_window_requests_a_compact_handoff_with_room_to_recover_truncation() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion(&candidate(), vec![]))]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.prepare_turn("Inspect only").unwrap();
    agent.messages.lock().unwrap().extend([
        Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("Earlier evidence. ".repeat(8_000))),
        ]),
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Continue the inspection")),
        ]),
    ]);
    let original = agent.messages.lock().unwrap().clone();
    agent
        .compact_if_needed(
            Some(Limits {
                context: 272_000,
                output: None,
            }),
            true,
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(*agent.messages.lock().unwrap(), original);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].body.encode().contains("under 16384 bytes"));
    assert_eq!(
        requests[0].body.get("max_completion_tokens"),
        Some(&Value::number(8192))
    );
}

#[test]
fn routing_charges_include_truncated_summary_attempts_and_their_recovery() {
    let charged = |mut response: Value, cost: f64| {
        let Value::Object(fields) = &mut response else {
            unreachable!()
        };
        fields.insert(
            "usage".into(),
            Value::object([("cost", Value::number(cost))]),
        );
        response
    };
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, charged(truncated(), 0.004)),
        (200, charged(completion(&candidate(), vec![]), 0.001)),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    // Reserve a fixed history margin beyond the native prompt and tool schema.
    agent.set_auto_compact(90);
    let capacity = agent.fixture_capacity(8_192, 90);
    agent.client.fixture_limits(capacity, None);
    historical_notes(&agent);
    agent.prepare_turn("Inspect only").unwrap();
    agent
        .compact_if_needed(
            Some(Limits {
                context: capacity,
                output: None,
            }),
            true,
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(agent.charges().requests, 2);
    assert!((agent.charges().cost - 0.005).abs() < 1e-9);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.body.get("tools").is_none())
    );
}

#[test]
fn learned_summary_allowance_survives_resume_and_avoids_repeated_truncation() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, truncated()),
        (200, completion(&candidate(), vec![])),
        (200, completion(&candidate(), vec![])),
    ]);
    let new_agent = || {
        let mut agent = Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        );
        agent.set_effort(Effort::High);
        // Keep output allowances while reserving room for native tools.
        agent.set_auto_compact(90);
        agent.enable_sessions(home.path()).unwrap();
        let capacity = agent.fixture_capacity(8_192, 90);
        agent.client.fixture_limits(capacity, None);
        agent
    };
    let mut first = new_agent();
    let capacity = first.fixture_capacity(8_192, 90);
    let limits = Some(Limits {
        context: capacity,
        output: None,
    });
    historical_notes(&first);
    first.prepare_turn("Inspect only").unwrap();
    let original = first.messages.lock().unwrap().clone();
    first
        .compact_if_needed(limits, true, &mut |_| Ok(()))
        .unwrap();
    let learned = first.context.summary_output_tokens;
    assert_eq!(*first.messages.lock().unwrap(), original);
    first.save_session().unwrap();
    let id = first.sessions().unwrap().id();
    drop(first);

    let mut resumed = new_agent();
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.context.summary_output_tokens, learned);
    historical_notes(&resumed);
    resumed
        .prepare_turn("Continue the same inspection")
        .unwrap();
    let original = resumed.messages.lock().unwrap().clone();
    resumed
        .compact_if_needed(limits, true, &mut |_| Ok(()))
        .unwrap();
    assert_eq!(resumed.context.summary_output_tokens, learned);
    assert_eq!(*resumed.messages.lock().unwrap(), original);
    let requests = fixture.finish();
    let allowances = requests
        .iter()
        .map(|request| {
            request
                .body
                .get("max_completion_tokens")
                .unwrap()
                .as_usize()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(allowances[0] <= 8_192);
    assert!(learned > allowances[0]);
    assert_eq!(allowances, [allowances[0], learned, learned]);
    for (request, allowance) in requests.iter().zip(allowances) {
        let messages = request.body.get("messages").unwrap().as_array().unwrap();
        let input = resumed
            .context
            .estimate_bytes(crate::context::bytes(messages).saturating_add(768));
        assert!(input.saturating_add(allowance) <= capacity);
    }
}

#[test]
fn learned_summary_allowance_respects_smaller_windows_and_provider_limits() {
    for (capacity, output, expected) in [
        (24000, None, 12000),
        (12000, None, 0),
        (24000, Some(4000), 4000),
    ] {
        let directory = Directory::new();
        let fixture = HttpFixture::new(vec![(200, completion(&candidate(), vec![]))]);
        let mut agent = Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        );
        agent.prepare_turn("Inspect only").unwrap();
        agent.context.summary_output_tokens = 12000;
        let original = agent.messages.lock().unwrap().clone();
        let records = original
            .iter()
            .enumerate()
            .skip(1)
            .map(|(index, message)| (index, message.encode()))
            .collect::<Vec<_>>();
        let (_, floor) = agent
            .summarize(
                &records,
                "Inspect only",
                Limits {
                    context: capacity,
                    output,
                },
                4096,
                &mut |_| Ok(()),
            )
            .unwrap();
        assert_eq!(floor, 12000);
        assert_eq!(*agent.messages.lock().unwrap(), original);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 1);
        let allowance = requests[0]
            .body
            .get("max_completion_tokens")
            .unwrap()
            .as_usize()
            .unwrap();
        if expected == 0 {
            assert!(allowance > capacity / 2 && allowance < capacity);
        } else {
            assert_eq!(allowance, expected);
        }
        let messages = requests[0]
            .body
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(crate::context::bytes(messages).saturating_add(768) <= capacity - allowance);
    }
}

#[test]
fn model_and_effort_changes_clear_only_the_learned_summary_allowance() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.context.summary_output_tokens = 6000;
    let original = agent.messages.lock().unwrap().clone();
    let memory = agent.context.summary.clone();
    agent.set_effort(Effort::Default);
    agent.set_model("fixture/model".into()).unwrap();
    assert_eq!(agent.context.summary_output_tokens, 6000);
    agent.set_effort(Effort::High);
    assert_eq!(agent.context.summary_output_tokens, 0);
    agent.context.summary_output_tokens = 6000;
    agent.set_model("fixture/other".into()).unwrap();
    assert_eq!(agent.context.summary_output_tokens, 0);
    agent.context.summary_output_tokens = 6000;
    agent.replace_client(OpenRouter::fixture(fixture.endpoint.clone()));
    assert_eq!(agent.context.summary_output_tokens, 0);
    agent.context.summary_output_tokens = 6000;
    agent.replace_client(OpenRouter::fixture(fixture.endpoint.clone()));
    assert_eq!(agent.context.summary_output_tokens, 6000);
    let mut replacement = OpenRouter::fixture(fixture.endpoint.clone());
    replacement.set_effort(Effort::High);
    agent.replace_client(replacement);
    assert_eq!(agent.context.summary_output_tokens, 0);
    assert_eq!(*agent.messages.lock().unwrap(), original);
    assert_eq!(agent.context.summary, memory);
    assert!(fixture.finish().is_empty());
}

#[test]
fn summary_allowance_reads_older_context_without_rewriting_it() {
    let mut value = crate::context::Context::default().value();
    if let Value::Object(fields) = &mut value {
        fields.remove("summary_output_tokens");
    }
    let original = value.clone();
    assert_eq!(
        crate::context::Context::parse(Some(&value))
            .unwrap()
            .summary_output_tokens,
        0
    );
    assert_eq!(value, original);
    if let Value::Object(fields) = &mut value {
        fields.insert("summary_output_tokens".into(), Value::string("invalid"));
    }
    assert!(crate::context::Context::parse(Some(&value)).is_err());
}

#[test]
fn output_regression_normal_turn_uses_declared_output_and_selected_effort() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Ready", vec![]))]);
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    // Leave room for the native tool catalog so this isolates the output ceiling.
    client.set_effort(Effort::High);
    let mut agent = Agent::new(client, Tools::new(directory.path()).unwrap());
    let capacity = agent.fixture_capacity(24_000, crate::context::AUTO_COMPACT_DEFAULT);
    agent.client.fixture_limits(capacity, Some(9000));
    agent.run_turn("Reply Ready", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].body.get("max_completion_tokens"),
        Some(&Value::number(9000))
    );
    assert_eq!(
        requests[0]
            .body
            .get("reasoning")
            .unwrap()
            .get("effort")
            .and_then(Value::as_str),
        Some("high")
    );
}

#[test]
fn output_regression_compaction_uses_a_modest_allowance_and_medium_effort() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion(&candidate(), vec![]))]);
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    client.set_effort(Effort::High);
    let mut agent = Agent::new(client, Tools::new(directory.path()).unwrap());
    agent.prepare_turn("Inspect only").unwrap();
    let original = agent.messages.lock().unwrap().clone();
    let records = original
        .iter()
        .enumerate()
        .skip(1)
        .map(|(index, message)| (index, message.encode()))
        .collect::<Vec<_>>();
    agent
        .summarize(
            &records,
            "Inspect only",
            Limits {
                context: 24000,
                output: Some(9000),
            },
            4096,
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(*agent.messages.lock().unwrap(), original);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].body.get("max_completion_tokens"),
        Some(&Value::number(3000))
    );
    assert_eq!(
        requests[0]
            .body
            .get("reasoning")
            .unwrap()
            .get("effort")
            .and_then(Value::as_str),
        Some("medium")
    );
    assert_eq!(agent.effort(), Effort::High);
    assert!(requests[0].body.get("tools").is_none());
}
