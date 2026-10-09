use super::*;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};

#[test]
fn resume_keeps_saved_model_calibration_but_discards_direct_request_measurements() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let new_agent = || {
        let client = crate::openrouter::OpenRouter::fixture(fixture.endpoint.clone());
        Agent::new(client, crate::tools::Tools::new(directory.path()).unwrap())
    };
    let mut first = new_agent();
    first.enable_sessions(home.path()).unwrap();
    first.prepare_turn("Continue the same work").unwrap();
    first.context.observe(
        Some(&Value::object([("prompt_tokens", Value::number(1000))])),
        2,
    );
    first.context.calibrate(6000);
    first.save_session().unwrap();
    let id = first.sessions().unwrap().id();
    drop(first);
    let mut resumed = new_agent();
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.context.calibration, Some((1000, 6000)));
    assert_eq!(resumed.context.input_tokens, None);
    assert_eq!(resumed.context.measured_end, 0);
    resumed.set_model("fixture/other".into()).unwrap();
    assert_eq!(resumed.context.calibration, None);
    assert!(fixture.finish().is_empty());
}

#[test]
fn native_protections_leave_room_for_carried_memory_in_a_small_context() {
    let directory = Directory::new();
    let home = Directory::new();
    let paths = (0..10)
        .map(|index| format!("keep-{index}.txt"))
        .collect::<Vec<_>>();
    for path in &paths {
        std::fs::write(directory.path().join(path), "original\r\nlast").unwrap();
    }
    let candidate = crate::context::memory::fixture("Continue the saved work.");
    let fixture = HttpFixture::new(vec![(200, completion(&candidate, vec![]))]);
    let mut client = crate::openrouter::OpenRouter::fixture(fixture.endpoint.clone());
    client.fixture_limits(32000, None);
    let mut agent = Agent::new(client, crate::tools::Tools::new(directory.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    agent
        .prepare_turn(&format!(
            "Preserve the originals exactly. {}",
            "User acceptance requirement. ".repeat(50)
        ))
        .unwrap();
    let arguments = Value::object([
        ("action", Value::string("record")),
        (
            "paths",
            Value::Array(paths.iter().map(Value::string).collect()),
        ),
        (
            "reason",
            Value::string(
                "Preserve existing acceptance files exactly, including original CRLF and final newline.",
            ),
        ),
    ]);
    let result = agent.tools.execute("protect", &arguments.encode());
    assert!(result.get("error").is_none());
    {
        let mut messages = agent.messages.lock().unwrap();
        messages.push(Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("")),
            (
                "tool_calls",
                Value::Array(vec![tool_call("record", "protect", arguments.clone())]),
            ),
        ]));
        messages.push(Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("record")),
            ("content", Value::string(result.encode())),
        ]));
    }
    agent.tools.record_file_history(3, &result);
    agent
        .context
        .evidence
        .record(3, "protect", &arguments, &result);
    let constraint = "Keep the exact acceptance contract. ".repeat(150);
    agent.context.summary = Value::object([
        ("objective", Value::string("Continue the ongoing change.")),
        (
            "constraints",
            Value::Array(vec![Value::string(&constraint)]),
        ),
        ("completed", Value::Array(vec![])),
        ("remaining", Value::Array(vec![])),
        (
            "next_action",
            Value::string("Continue the saved next action. ".repeat(30)),
        ),
    ])
    .encode();
    agent.context.from = 4;
    agent.context.observe(
        Some(&Value::object([
            ("prompt_tokens", Value::number(28000)),
            ("completion_tokens", Value::number(5000)),
        ])),
        4,
    );
    let cancellation = agent.cancellation();
    assert!(
        agent
            .compact_if_needed(
                Some(Limits {
                    context: 32000,
                    output: None
                }),
                false,
                &mut |event| {
                    if matches!(event, Event::Maintenance { ref text } if text.contains("Repairing"))
                        || matches!(event, Event::Recovering { .. })
                    {
                        cancellation.cancel();
                    }
                    Ok(())
                }
            )
            .unwrap_or_else(|error| panic!("{error}; {:?}", agent.events.lock().unwrap()))
    );
    assert!(agent.context.summary.contains(&constraint));
    assert_eq!(
        agent
            .context
            .evidence
            .value()
            .get("file_constraints")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        10
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].body.get("tools").is_none());
    assert!(requests[0].body.encode().contains("Return at most"));
}
