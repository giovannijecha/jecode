use super::*;
use crate::{events::Event, json::Value};

fn start(state: &mut State, id: &str) {
    state.event(Event::ToolStarted {
        id: id.into(),
        name: "read".into(),
        arguments: Value::object([("path", Value::string(format!("{id}.rs")))]),
    });
}

#[test]
fn a_finished_turn_expands_pending_members_together_with_completed_calls() {
    let mut state = State::default();
    state.begin_tool_turn();
    start(&mut state, "unresolved");
    start(&mut state, "finished");
    state.event(Event::ToolFinished {
        id: "finished".into(),
        name: "read".into(),
        summary: "source captured".into(),
        result: Value::object([("content", Value::string("captured source"))]),
    });
    state.close_tools();
    state.finish_tool_turn();

    assert_eq!(state.toggle_tools(), Some((true, 2)));
    assert!(
        state
            .items
            .iter()
            .filter_map(Item::tool)
            .all(|tool| tool.presentation.expanded)
    );
    let rows: Vec<_> = state.rows().into_iter().flatten().collect();
    assert!(!rows.iter().any(|line| line.plain().starts_with("  WORK  ")));
    assert!(
        rows.iter()
            .any(|line| line.plain().contains("unresolved.rs"))
    );
    assert!(
        rows.iter()
            .any(|line| line.plain().contains("captured source"))
    );

    assert_eq!(state.toggle_tools(), Some((false, 2)));
    assert!(
        state
            .items
            .iter()
            .filter_map(Item::tool)
            .all(|tool| !tool.presentation.expanded)
    );
    assert!(
        state
            .rows()
            .into_iter()
            .flatten()
            .any(|line| line.plain().starts_with("  WORK  "))
    );
}
