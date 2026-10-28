use super::*;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};

fn measured_growth_agent(directory: &Directory, fixture: &HttpFixture) -> (Agent, Vec<Value>) {
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    client.fixture_limits(45000, None);
    let mut agent = Agent::new(client, Tools::new(directory.path()).unwrap());
    agent
        .prepare_turn("Continue the current inspection")
        .unwrap();
    agent.context.observe(
        Some(&crate::backend::Usage {
            input_tokens: Some(16635),
            ..Default::default()
        }),
        2,
    );
    agent.context.calibrate(73239);
    agent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("assistant")),
        (
            "content",
            Value::string("Recorded inspection data ".repeat(960)),
        ),
    ]));
    let original = agent.messages.lock().unwrap().clone();
    // Raw growth crosses the default 80% budget; the calibrated estimate does not.
    let budget = crate::context::threshold(45000, crate::context::AUTO_COMPACT_DEFAULT);
    assert!(16635 + crate::context::bytes(&original[2..]) > budget);
    assert!(agent.context_estimate(&original) < budget);
    (agent, original)
}

#[test]
fn calibrated_growth_avoids_a_summary_and_keeps_original_history_in_the_request() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Inspection finished.", vec![]))]);
    let (mut agent, original) = measured_growth_agent(&directory, &fixture);
    agent
        .run_turn("Continue the current inspection", &mut |_| Ok(()))
        .unwrap();
    assert_eq!(agent.context.from, 1);
    assert!(agent.context.summary.is_empty());
    assert_eq!(agent.messages.lock().unwrap()[..original.len()], original);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].body.get("tools").is_some());
    assert!(
        requests[0]
            .body
            .encode()
            .contains("Recorded inspection data")
    );
}

#[test]
fn a_rejected_calibrated_growth_request_recovers_with_native_compaction() {
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
            completion(&String::from("Keep the current inspection"), vec![]),
        ),
        (
            200,
            completion(&String::from("Keep the current inspection"), vec![]),
        ),
        (200, completion("Inspection finished.", vec![])),
    ]);
    let (mut agent, _) = measured_growth_agent(&directory, &fixture);
    // A rejected request learns 75% of the uncalibrated estimate. Keep that
    // ceiling near the advertised window so the fixed native catalog leaves
    // summary room, while the previous assistant history can be compacted.
    let raw = {
        let mut uncalibrated = agent.context.clone();
        uncalibrated.calibration = None;
        let messages = agent.messages.lock().unwrap();
        let measured = uncalibrated
            .measured_input(&messages, agent.client.inputs())
            .unwrap();
        uncalibrated.estimate_growth(&messages, measured, agent.client.inputs())
    };
    let extra = 45_000usize.saturating_sub(raw).saturating_add(512);
    const NOTE: &str = "Earlier inspection note. ";
    agent.messages.lock().unwrap().insert(
        2,
        Value::object([
            ("role", Value::string("assistant")),
            (
                "content",
                Value::string(NOTE.repeat(extra.div_ceil(NOTE.len()))),
            ),
        ]),
    );
    let original = agent.messages.lock().unwrap().clone();
    agent.set_auto_compact(99);
    assert!(agent.context_estimate(&original) < crate::context::threshold(45_000, 99));
    agent
        .run_turn("Continue the current inspection", &mut |_| Ok(()))
        .unwrap();
    assert!(agent.context.ceiling.is_some());
    assert!(agent.context.from > 1);
    assert_eq!(agent.messages.lock().unwrap()[..original.len()], original);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 4);
    assert!(requests[0].body.get("tools").is_some());
    assert!(requests[1].body.get("tools").is_none());
    assert!(requests[2].body.get("tools").is_none());
    assert!(requests[3].body.get("tools").is_some());
    assert!(
        requests[3]
            .body
            .encode()
            .contains("Keep the current inspection")
    );
}

#[test]
fn a_provider_context_rejection_compacts_and_retries_without_losing_the_prompt() {
    let directory = Directory::new();
    // Leave summary room after a rejection of the native prompt and tool contract.
    let prompt = "original objective ".repeat(1500);
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
            completion(
                &String::from("Original objective retained, including the last portion."),
                vec![],
            ),
        ),
        (
            200,
            completion(
                "Original objective retained, including the last portion.",
                vec![],
            ),
        ),
        (200, completion("Finished.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let fixed = agent.current_system_message().encode().len()
        + crate::tools::encoded_size()
        + agent.request_environment().len()
        + prompt.len();
    agent
        .messages
        .lock()
        .unwrap()
        .extend(crate::test_support::prior_compactable_exchange(fixed));
    agent.set_auto_compact(90);
    agent
        .run_turn(&prompt, &mut |event| {
            if matches!(event, Event::Recovering { .. }) {
                return Err("Unexpected fixture request".into());
            }
            Ok(())
        })
        .unwrap();
    assert!(agent.context.ceiling.is_some());
    assert!(agent.messages.lock().unwrap().iter().any(|message| {
        message.get("role").and_then(Value::as_str) == Some("user")
            && message.get("content").and_then(Value::as_str) == Some(prompt.as_str())
    }));
    let requests = fixture.finish();
    assert_eq!(requests.len(), 4);
    assert!(
        requests[1..3]
            .iter()
            .all(|request| request.body.get("tools").is_none())
    );
    assert!(requests[3].body.get("tools").is_some());
    assert!(requests[3].body.encode().contains("last portion"));
    assert_eq!(
        agent
            .messages
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .get("content")
            .and_then(Value::as_str),
        Some("Finished.")
    );
}

#[test]
fn an_unsaved_summary_is_not_applied_and_no_following_tool_runs() {
    let directory = Directory::new();
    let home = Directory::new();
    std::fs::write(directory.path().join("source"), "evidence\n".repeat(2000)).unwrap();
    let mut first = completion(
        "",
        vec![tool_call(
            "read",
            "read",
            Value::object([
                ("path", Value::string("source")),
                ("limit", Value::number(2000)),
            ]),
        )],
    );
    if let Value::Object(fields) = &mut first {
        // Budget-relative synthetic usage keeps compaction at the post-read boundary as the catalog grows.
        fields.insert(
            "usage".into(),
            Value::object([(
                "prompt_tokens",
                Value::number(6000 + crate::tools::encoded_size()),
            )]),
        );
    }
    let fixture = HttpFixture::new(vec![
        (200, first),
        (
            200,
            completion(
                &String::from("Goal and original evidence preserved."),
                vec![],
            ),
        ),
    ]);
    let client = OpenRouter::fixture(fixture.endpoint.clone());
    let mut agent = Agent::new(client, Tools::new(directory.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    let capacity = agent.fixture_capacity(6_000, 80);
    agent.client.fixture_limits(capacity, Some(4096));
    agent.prepare_turn("Inspect").unwrap();
    let handle = agent.sessions().unwrap();
    let bucket = std::fs::read_dir(home.path().join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let path = bucket.join(format!("{}.jsonl", handle.id()));
    let mut original_permissions = None;
    let error = agent
        .run_turn("Inspect", &mut |event| {
            if matches!(event, Event::Maintenance { .. }) {
                let original = std::fs::metadata(&path).unwrap().permissions();
                let mut readonly = original.clone();
                readonly.set_readonly(true);
                std::fs::set_permissions(&path, readonly).unwrap();
                original_permissions = Some(original);
            }
            Ok(())
        })
        .unwrap_err();
    assert!(original_permissions.is_some(), "{error}");
    std::fs::set_permissions(&path, original_permissions.unwrap()).unwrap();
    assert!(error.contains("read-only"));
    assert_eq!(agent.context.from, 1);
    assert!(agent.context.summary.is_empty());
    agent.save_session().unwrap();
    let saved = handle.store().fixture_load(&handle.id()).unwrap();
    assert_eq!(saved.context.from, 1);
    assert_eq!(saved.messages.len(), 4);
    assert!(saved.messages[3].encode().contains("evidence"));
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn carried_memory_can_exceed_eight_kib_while_fitting_the_active_context() {
    let directory = Directory::new();
    let constraint = format!("Preserve the complete requirement: {}", "c".repeat(9000));
    let memory = Value::object([
        ("objective", Value::string("Continue the original work")),
        (
            "constraints",
            Value::Array(vec![Value::string(&constraint)]),
        ),
        ("completed", Value::Array(vec![])),
        (
            "remaining",
            Value::Array(vec![Value::string("Run the pending check")]),
        ),
        ("next_action", Value::string("Run the pending check")),
    ])
    .encode();
    assert!(memory.len() > 8192);
    let fixture = HttpFixture::new(vec![
        (200, completion(&memory, vec![])),
        (200, completion("Finished.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.context.summary = memory;
    // Preserve room for this >8 KiB carried memory within the active context.
    let capacity = agent.fixture_capacity(agent.context.summary.len() + 16_384, 80);
    agent.client.fixture_limits(capacity, Some(4096));
    agent.messages.lock().unwrap().extend([
        Value::object([
            ("role", Value::string("user")),
            (
                "content",
                Value::string("Continue and preserve the carried requirement."),
            ),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("a".repeat(14000))),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("b".repeat(14000))),
        ]),
    ]);
    agent
        .run_turn("Continue with the pending check.", &mut |_| Ok(()))
        .unwrap();
    let saved = crate::json::parse(&agent.context.summary).unwrap();
    assert_eq!(
        saved.get("constraints").unwrap().as_array().unwrap(),
        &[Value::string(constraint)]
    );
    assert!(agent.context.summary.len() > 8192);
    assert!(agent.context_status().tokens < capacity);
    assert_eq!(agent.messages.lock().unwrap().len(), 6);
    assert_eq!(
        agent
            .messages
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .get("content"),
        Some(&Value::string("Finished."))
    );
    assert_eq!(fixture.finish().len(), 2);
}
