use super::*;

fn command_record(result: Value, summary: &str) -> Value {
    let mut agent = record(1, "returned");
    set(
        &mut agent,
        "tools",
        Value::Array(vec![Value::object([
            ("name", Value::string("exec")),
            ("text", Value::string(summary)),
            ("result", result),
        ])]),
    );
    // The structured result takes precedence over an ambiguous old summary.
    set(
        &mut agent,
        "checks",
        Value::Array(vec![Value::string("exec: no failures")]),
    );
    agent
}

#[test]
fn structured_command_outcomes_classify_checks() {
    let cases = [
        (Value::object([("exit_code", Value::number(0))]), false),
        (Value::object([("exit_code", Value::number(1))]), true),
        (
            Value::object([("error", Value::string("could not start"))]),
            true,
        ),
        (Value::object([("timed_out", Value::Bool(true))]), true),
        (Value::object([("cancelled", Value::Bool(true))]), true),
        (Value::object([("status", Value::string("running"))]), false),
    ];
    for (result, failed) in cases {
        let agent = command_record(result, "exit 1");
        assert_eq!(failed_check(&agent).is_some(), failed);
        assert_eq!(status(&agent) == "failed check · main review", failed);
    }
}

#[test]
fn legacy_summaries_remain_readable_without_structured_results() {
    let mut agent = record(1, "returned");
    for (summary, failed) in [
        ("bash: exit 0", false),
        ("bash: exit 1", true),
        ("exec: exit 0; output truncated", false),
        ("bash: exit 1; full output saved", true),
        ("exec: timed out", true),
        ("bash: search check failed", true),
    ] {
        set(
            &mut agent,
            "checks",
            Value::Array(vec![Value::string(summary)]),
        );
        assert_eq!(failed_check(&agent).is_some(), failed, "{summary}");
    }
}

#[test]
fn failed_check_marks_agent_list_receipt_and_footer() {
    let agent = command_record(Value::object([("exit_code", Value::number(1))]), "exit 1");
    let mut state = State {
        width: 100,
        height: 24,
        collaboration: vec![agent.clone()],
        ..State::default()
    };
    let footer = text(&view::composer_frame(&state, "fixture/model", "fixture").live);
    assert!(footer.contains("1 failed check"), "{footer}");
    state.agents_panel = Some(Panel::default());
    let list = text(&view::composer_frame(&state, "fixture/model", "fixture").live);
    assert!(list.contains("failed check · main review"), "{list}");
    state.agents_panel = None;
    state.collaboration_event(event(1, "returned", 1, vec![agent]));
    let receipt = all(&state);
    assert!(receipt.contains("check failed · exit 1"), "{receipt}");
}
