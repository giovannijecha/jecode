//! A completed task keeps the statistics of its real work, including delegation.

use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    json::Value,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};

fn reported(mut response: Value, output: usize, cached: usize) -> Value {
    if let Value::Object(fields) = &mut response {
        fields.insert(
            "usage".into(),
            Value::object([
                ("prompt_tokens", Value::number(12_000)),
                ("completion_tokens", Value::number(output)),
                (
                    "prompt_tokens_details",
                    Value::object([("cached_tokens", Value::number(cached))]),
                ),
            ]),
        );
    }
    response
}

#[test]
fn returned_collaborator_already_reviewed_does_not_replace_completion_statistics() {
    let home = Directory::new();
    let project = Directory::new();
    let create = Value::object([
        ("action", Value::string("create")),
        (
            "tasks",
            Value::Array(vec![Value::object([
                ("title", Value::string("Inspect notes")),
                ("instructions", Value::string("Report the findings")),
                ("workspace", Value::string("shared")),
            ])]),
        ),
        ("work", Value::string("Review notes")),
    ]);
    let wait = Value::object([
        ("action", Value::string("wait")),
        ("id", Value::string("a1")),
        ("work", Value::string("Review notes")),
    ]);
    // Waiting in the same batch orders the child's response before the
    // main agent's final response, which already consumes the return.
    let fixture = HttpFixture::new(vec![
        (
            200,
            reported(
                completion(
                    "",
                    vec![
                        tool_call("delegate", "agent", create),
                        tool_call("wait", "agent", wait),
                    ],
                ),
                1_000,
                5_000,
            ),
        ),
        (200, reported(completion("Child findings.", vec![]), 200, 0)),
        (
            200,
            reported(completion("Reviewed all findings.", vec![]), 3_000, 6_000),
        ),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let mut app = App::new(agent, tests::config(&project), None);
    app.state.width = 120;
    app.dispatch("Inspect and review".into()).unwrap();
    app.state.activity = Some(activity::Activity::with_elapsed(Duration::from_secs(64)));
    tests::finish(&mut app);

    let ending = app.state.ending.as_ref().unwrap().text.clone();
    assert!(ending.contains("1m 04s"), "{ending}");
    assert!(ending.contains("2 tools"), "{ending}");
    assert!(ending.contains("↓ 4.0k"), "{ending}");
    assert!(ending.contains("11K cached"), "{ending}");
    let drawn = view::composer_frame(&app.state, "fixture/model", "fixture")
        .live
        .iter()
        .map(line::Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    for fact in ["Done", "1m 04s", "2 tools", "↓ 4.0k"] {
        assert!(drawn.contains(fact), "{drawn}");
    }
    // Further idle polls cannot turn the consumed receipt into a fresh,
    // zero-duration worker or overwrite the finished status.
    for _ in 0..4 {
        app.poll().unwrap();
        assert!(app.worker.is_none());
        assert_eq!(app.state.ending.as_ref().unwrap().text, ending);
    }
    assert!(!app.agent.as_ref().unwrap().can_continue_collaboration());
    assert_eq!(fixture.finish().len(), 3);
}
