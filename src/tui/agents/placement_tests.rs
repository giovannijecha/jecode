//! Where agent events land: from what the runtime recorded, never from
//! when the interface happens to see them, and the same after resume.

use super::*;
use crate::test_support::tool_call;

fn started(id: &str, name: &str, arguments: Value) -> Event {
    Event::ToolStarted {
        id: id.into(),
        name: name.into(),
        arguments,
    }
}

fn finished(id: &str, name: &str) -> Event {
    Event::ToolFinished {
        id: id.into(),
        name: name.into(),
        summary: "done".into(),
        result: Value::object([]),
    }
}

fn read(id: &str, work: &str) -> Value {
    Value::object([
        ("path", Value::string(format!("src/{id}.rs"))),
        ("work", Value::string(work)),
    ])
}

/// The event as the runtime recorded it inside or outside a turn and a
/// call, written by no agent call.
fn recorded(mut event: Value, in_turn: bool, in_call: bool) -> Value {
    set(&mut event, "in_turn", Value::Bool(in_turn));
    set(&mut event, "in_call", Value::Bool(in_call));
    event
}

/// Each call or event with its chapter label, whether it stands alone and
/// its Ctrl+O turn, numbered by first appearance.
fn placement(items: &[Item]) -> Vec<(String, String, bool, usize)> {
    let mut turns = Vec::new();
    items
        .iter()
        .filter_map(|item| {
            let Item::Tool {
                name,
                arguments,
                presentation,
                ..
            } = item
            else {
                return None;
            };
            let turn = turns
                .iter()
                .position(|turn| *turn == presentation.turn_id)
                .unwrap_or_else(|| {
                    turns.push(presentation.turn_id);
                    turns.len() - 1
                });
            Some((
                name.clone(),
                value(arguments, "work").to_owned(),
                arguments.get("lone") == Some(&Value::Bool(true)),
                turn,
            ))
        })
        .collect()
}

#[test]
fn a_receipt_polled_before_its_call_starts_still_joins_that_call() {
    let shown = |receipt_first: bool| {
        let home = Directory::new();
        let directory = Directory::new();
        let mut app = super::replay_tests::replay_app(&home, &directory);
        app.state.width = 100;
        app.state.begin_tool_turn();
        for id in ["r1", "r2"] {
            app.worker_event(started(id, "read", read(id, "Check the docs")));
            app.worker_event(finished(id, "read"));
        }
        let sent = labelled(
            event(1, "sent", 1, vec![record(1, "running")]),
            "Update agents",
        );
        let call = started(
            "u1",
            "agent",
            Value::object([
                ("action", Value::string("update")),
                ("id", Value::string("a1")),
                ("work", Value::string("Update agents")),
            ]),
        );
        if receipt_first {
            // The observer is polled before the worker's start event arrives.
            assert!(app.show_collaboration(sent));
            app.worker_event(call);
        } else {
            app.worker_event(call);
            app.worker_event(Event::Collaboration { event: sent });
        }
        app.worker_event(finished("u1", "agent"));
        app.worker_event(started("r3", "read", read("r3", "Update agents")));
        app.worker_event(finished("r3", "read"));
        (placement(&app.state.items), all(&app.state))
    };
    let (placed, rows) = shown(true);
    assert_eq!((placed, rows.clone()), shown(false));
    assert!(rows.contains("WORK  Check the docs  2 calls"), "{rows}");
    assert!(rows.contains("WORK  Update agents  2 calls"), "{rows}");
}

#[test]
fn resumed_events_take_the_places_they_had_live() {
    let home = Directory::new();
    let directory = Directory::new();
    let work = "Check the source";
    // A child returns while the first read runs, another after the turn,
    // and a third is still running when the session closes.
    let during = recorded(
        event(1, "returned", 1, vec![record(1, "returned")]),
        true,
        true,
    );
    let late = recorded(
        event(
            2,
            "returned",
            2,
            vec![
                record(1, "returned"),
                record(2, "returned"),
                record(3, "running"),
            ],
        ),
        false,
        false,
    );
    let current = super::replay_tests::replay_app(&home, &directory);
    let archive = current.agent.as_ref().unwrap().archive();
    let mut messages = archive.messages.lock().unwrap();
    let mut events = archive.events.lock().unwrap();
    events.push(Value::object([
        ("type", Value::string("turn_start")),
        ("after_message", Value::number(messages.len())),
    ]));
    let text = |role: &str, text: &str| {
        Value::object([
            ("role", Value::string(role)),
            ("content", Value::string(text)),
        ])
    };
    let calls = |id: &str, work: &str| {
        Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(id, "read", read(id, work))]),
            ),
        ])
    };
    let result = |id: &str| {
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string(id)),
            ("content", Value::string("{}")),
        ])
    };
    messages.push(text("user", "Check the source"));
    messages.push(calls("r1", work));
    let mut saved = during.clone();
    set(&mut saved, "after_message", Value::number(messages.len()));
    events.push(saved);
    messages.extend([result("r1"), calls("r2", work), result("r2")]);
    messages.push(text("assistant", "Both checked."));
    let mut saved = late.clone();
    set(&mut saved, "after_message", Value::number(messages.len()));
    events.push(saved);
    // The automatic review that followed the late return.
    messages.extend([calls("r3", "Review the return"), result("r3")]);
    messages.push(text("assistant", "Reviewed."));
    drop((messages, events));
    current.agent.as_ref().unwrap().save_session().unwrap();
    let id = current.persistence.as_ref().unwrap().id();
    drop(current);

    let mut resumed = super::replay_tests::replay_app(&home, &directory);
    resumed.resume_session(&id);
    resumed.state.width = 100;
    let unknown = resumed
        .state
        .items
        .iter()
        .find_map(|item| match item {
            Item::Tool {
                result: Some(event),
                ..
            } if value(event, "kind") == "unknown" => Some(event.clone()),
            _ => None,
        })
        .expect("resume records the interrupted collaborator");
    assert_eq!(unknown.get("in_turn"), Some(&Value::Bool(false)));

    let mut live = State {
        width: 100,
        ..State::default()
    };
    let user = crate::tui::state::Kind::User;
    let assistant = crate::tui::state::Kind::Assistant;
    live.begin_tool_turn();
    live.message(user, "Check the source");
    live.event(started("r1", "read", read("r1", work)));
    live.collaboration_event(during);
    live.event(finished("r1", "read"));
    live.event(started("r2", "read", read("r2", work)));
    live.event(finished("r2", "read"));
    live.message(assistant, "Both checked.");
    live.finish_tool_turn();
    live.collaboration_event(late);
    live.begin_tool_turn();
    live.event(started("r3", "read", read("r3", "Review the return")));
    live.event(finished("r3", "read"));
    live.message(assistant, "Reviewed.");
    live.finish_tool_turn();
    live.collaboration_event(unknown);

    let placed = placement(&resumed.state.items);
    assert_eq!(placed, placement(&live.items));
    let lone = |kind: &str| {
        resumed.state.items.iter().any(|item| {
            matches!(item, Item::Tool { arguments, result: Some(event), .. }
                if value(event, "kind") == kind
                    && arguments.get("lone") == Some(&Value::Bool(true)))
        })
    };
    assert!(lone("unknown") && lone("returned"), "{placed:?}");
    let rows = all(&resumed.state);
    assert!(rows.contains("WORK  Check the source  3 calls"), "{rows}");
}
