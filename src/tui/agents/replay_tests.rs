use super::*;

fn replay_app(home: &Directory, directory: &Directory) -> App {
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    App::new(
        agent,
        SessionConfig {
            store: Store::new(home.path().join("config")),
            settings,
            bash: crate::tools::find_bash().unwrap(),
        },
        None,
    )
}
#[test]
fn resumed_agent_workspace_cards_match_live_actions_and_keep_errors() {
    let home = Directory::new();
    let directory = Directory::new();
    let current = replay_app(&home, &directory);
    let archive = current.agent.as_ref().unwrap().archive();
    let mut messages = archive.messages.lock().unwrap();
    messages.push(Value::object([
        ("role", Value::string("user")),
        (
            "content",
            Value::string("Inspect and integrate the agent work"),
        ),
    ]));
    for (id, action, result) in [
        (
            "diff-1",
            "diff",
            Value::object([("diff", Value::string("file changed"))]),
        ),
        (
            "integrate-1",
            "integrate",
            Value::object([("checkpoint", Value::string("abc123"))]),
        ),
        (
            "review-1",
            "review",
            Value::object([("error", Value::string("Review failed"))]),
        ),
    ] {
        let arguments = Value::object([
            ("action", Value::string(action)),
            ("id", Value::string("a1")),
        ]);
        messages.push(Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(id, "agent", arguments)]),
            ),
        ]));
        messages.push(Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string(id)),
            ("content", Value::string(result.encode())),
        ]));
    }
    messages.push(Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string("Integration inspected")),
    ]));
    drop(messages);
    current.agent.as_ref().unwrap().save_session().unwrap();
    let id = current.persistence.as_ref().unwrap().id();
    drop(current);

    let mut resumed = replay_app(&home, &directory);
    resumed.resume_session(&id);
    let cards = resumed
        .state
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Tool {
                arguments, result, ..
            } => Some((
                arguments
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_owned(),
                result.as_ref().unwrap().clone(),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[0].0, "diff");
    assert_eq!(cards[1].0, "integrate");
    assert_eq!(
        cards[1].1.get("checkpoint").and_then(Value::as_str),
        Some("abc123")
    );
    assert!(resumed.state.items.iter().any(|item| matches!(item,
        Item::Text { kind: crate::tui::state::Kind::Error, text } if text.contains("Review failed")
    )));
}
