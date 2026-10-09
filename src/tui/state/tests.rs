use super::*;
use crate::tui::line::Line;

fn start(state: &mut State, id: &str) {
    state.event(Event::ToolStarted {
        id: id.into(),
        name: "bash".into(),
        arguments: Value::object([("command", Value::string("fixture-command"))]),
    });
}
fn finish(state: &mut State, id: &str, output: &str) {
    state.event(Event::ToolFinished {
        id: id.into(),
        name: "bash".into(),
        summary: "exit 0".into(),
        result: Value::object([
            ("stdout", Value::string(output)),
            ("exit_code", Value::number(0)),
        ]),
    });
}
fn visible(state: &State) -> String {
    state
        .rows()
        .iter()
        .flatten()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn every_turn_uses_short_previews_and_keeps_its_original_results() {
    let mut state = State::default();
    state.message(Kind::User, "Inspect the fixture.");
    for _ in 0..2 {
        start(&mut state, "same");
        finish(&mut state, "same", "one\ntwo\nthree\nfour\nretained detail");
    }
    state.message(Kind::Assistant, &"newer message\n".repeat(40));
    state.message(Kind::Notice, "Saved fixture export.");
    state.message(Kind::User, "Another request.");
    start(&mut state, "future");
    assert!(
        visible(&state)
            .lines()
            .any(|row| row.contains("bash  fixture-command") && row.contains("running"))
    );
    finish(&mut state, "future", "one\ntwo\nthree\nfour\nfuture detail");
    let text = visible(&state);
    assert_eq!(text.matches("✓ exit 0").count(), 3);
    assert_eq!(text.matches("preview 3 / 5").count(), 2);
    assert!(!text.contains("omitted"));
    assert!(text.contains("retained detail"));
    assert!(text.contains("future detail"));
    assert!(!text.lines().any(|row| row.trim() == "one"));
    for item in &state.items {
        if let Item::Tool {
            result: Some(result),
            ..
        } = item
        {
            let capture = result.get("stdout").and_then(Value::as_str).unwrap();
            assert!(capture.ends_with("detail"));
        }
    }
}

#[test]
fn results_match_the_latest_unfinished_call_without_overwriting_earlier_results() {
    let mut state = State::default();
    start(&mut state, "repeated");
    finish(&mut state, "repeated", "first result");
    start(&mut state, "repeated");
    finish(&mut state, "repeated", "second result");
    finish(&mut state, "repeated", "unexpected duplicate");
    assert_eq!(state.items.len(), 2);
    state.select_tool(Some(0));
    state.toggle_tool();
    state.select_tool(None);
    let text = visible(&state);
    assert!(text.contains("first result"));
    assert!(text.contains("second result"));
    assert!(!text.contains("unexpected duplicate"));
}

#[test]
fn messages_are_distinguished_by_presentation_and_keep_their_original_content() {
    let mut state = State::default();
    state.message(Kind::User, "my request");
    state.message(Kind::Assistant, "**response**");
    let rows = state.rows();
    assert!(
        !rows
            .iter()
            .flatten()
            .any(|row| matches!(row.plain().as_str(), "You" | "Jecode"))
    );
    assert!(
        rows[0]
            .iter()
            .any(|row| row.plain().trim() == "my request" && row.background.is_some())
    );
    assert!(
        rows[1]
            .iter()
            .any(|row| row.plain() == "response" && row.background.is_none())
    );
    assert!(matches!(&state.items[1], Item::Text { text, .. } if text == "**response**"));
}

#[test]
fn clearing_changes_the_display_generation_without_discarding_an_unsent_draft() {
    let mut state = State::default();
    state.editor.insert("kept draft");
    start(&mut state, "old");
    let generation = state.generation;
    state.clear();
    assert!(state.items.is_empty());
    assert_ne!(state.generation, generation);
    assert_eq!(state.editor.text, "kept draft");
    start(&mut state, "new");
    finish(&mut state, "new", "fresh preview");
    assert!(visible(&state).contains("fresh preview"));
}

#[test]
fn consecutive_tools_connect_across_model_requests_and_close_at_a_visible_message() {
    let mut state = State::default();
    start(&mut state, "first");
    finish(&mut state, "first", "first result");
    assert!(matches!(&state.items[0], Item::Tool { last: None, .. }));
    let waiting = state.rows();
    state.event(Event::Waiting {
        model: "fixture".into(),
    });
    assert_eq!(state.rows(), waiting);
    start(&mut state, "second");
    assert!(matches!(
        &state.items[0],
        Item::Tool {
            last: Some(false),
            ..
        }
    ));
    finish(&mut state, "second", "second result");
    state.event(Event::Waiting {
        model: "fixture".into(),
    });
    assert!(matches!(&state.items[1], Item::Tool { last: None, .. }));
    let first = state.rows()[0].clone();
    start(&mut state, "third");
    finish(&mut state, "third", "third result");
    state.event(Event::Waiting {
        model: "fixture".into(),
    });
    state.message(Kind::Assistant, "Finished.");
    let current = state.rows();
    assert_eq!(current[0], first);
    assert!(current[0][0].plain().starts_with("├─ bash"));
    assert!(current[1][0].plain().starts_with("├─ bash"));
    assert!(current[2][0].plain().starts_with("└─ bash"));
    start(&mut state, "later");
    finish(&mut state, "later", "later result");
    state.close_tools();
    assert_eq!(state.rows()[..4], current);
}

#[test]
fn a_notice_closes_the_tool_branch_without_losing_its_pending_result() {
    let mut state = State::default();
    start(&mut state, "first");
    state.message(Kind::Notice, "Saved fixture export.");
    assert!(matches!(
        &state.items[0],
        Item::Tool {
            last: Some(true),
            result: None,
            ..
        }
    ));
    finish(&mut state, "first", "retained result");
    let settled = state.rows();
    start(&mut state, "second");
    state.close_tools();
    assert_eq!(state.rows()[..1], settled);
    assert!(visible(&state).contains("retained result"));
}
