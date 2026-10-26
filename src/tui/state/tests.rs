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
fn earlier_chapter_and_latest_singleton_keep_compact_previews_and_original_results() {
    let mut state = State::default();
    state.message(Kind::User, "Inspect the fixture.");
    for _ in 0..2 {
        start(&mut state, "same");
        finish(&mut state, "same", "one\ntwo\nthree\nfour\nretained detail");
    }
    state.message(Kind::Assistant, &"newer message\n".repeat(40));
    state.message(Kind::Notice, "Saved fixture export.");
    state.message(Kind::User, "Another request.");
    state.begin_tool_turn();
    start(&mut state, "future");
    let running = visible(&state);
    assert!(running.contains("● Activity  2 calls"));
    assert!(running.contains("● Bash(fixture-command)"));
    assert!(running.contains("⎿ ") && running.contains('●'));
    assert!(!running.contains(['▰', '▱']));
    finish(&mut state, "future", "one\ntwo\nthree\nfour\nfuture detail");
    let text = visible(&state);
    assert!(text.contains("2 commands") && text.contains("● Bash(fixture-command)"));
    assert!(!text.contains("omitted"));
    assert!(!text.contains("retained detail"));
    assert!(text.contains("future detail")); // The singleton keeps its normal result preview.
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
    state.close_tools();
    state.finish_tool_turn();
    state.toggle_tools().unwrap();
    assert!(
        visible(&state)
            .lines()
            .any(|row| row == "    future detail")
    );
}

#[test]
fn results_match_the_latest_unfinished_call_without_overwriting_earlier_results() {
    let mut state = State::default();
    state.begin_tool_turn();
    start(&mut state, "repeated");
    finish(&mut state, "repeated", "first result");
    start(&mut state, "repeated");
    finish(&mut state, "repeated", "second result");
    finish(&mut state, "repeated", "unexpected duplicate");
    assert_eq!(state.items.len(), 2);
    state.finish_tool_turn();
    state.toggle_tools();
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
            .any(|row| row.plain() == "● response" && row.background.is_none())
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
    state.begin_tool_turn();
    start(&mut state, "new");
    finish(&mut state, "new", "fresh preview");
    assert!(visible(&state).contains("fresh preview"));
    assert!(
        !visible(&state)
            .lines()
            .any(|row| row == "    fresh preview")
    );
    state.close_tools();
    state.finish_tool_turn();
    state.toggle_tools().unwrap();
    assert!(visible(&state).contains("fresh preview"));
}

#[test]
fn consecutive_calls_share_a_chapter_across_model_requests_and_keep_targets() {
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
    assert_ne!(current[0], first);
    assert!(current[0][0].plain().starts_with("● Activity  3 calls"));
    assert!(
        current[0]
            .iter()
            .any(|row| row.plain().contains("3 commands"))
    );
    assert!(current[1][0].plain().contains("Bash  fixture-command"));
    assert!(current[2][0].plain().contains("Bash  fixture-command"));
    assert!(
        !current[1..3]
            .iter()
            .flatten()
            .any(|row| row.plain().contains("result"))
    );
    for (item, output) in
        state.items[..3]
            .iter()
            .zip(["first result", "second result", "third result"])
    {
        assert!(matches!(item, Item::Tool { result: Some(result), .. }
            if result.get("stdout").and_then(Value::as_str) == Some(output)));
    }
    start(&mut state, "later");
    finish(&mut state, "later", "later result");
    state.close_tools();
    assert_eq!(state.rows()[..4], current);
}

#[test]
fn captured_file_diff_stays_visible_when_the_next_tool_starts() {
    let mut state = State::default();
    let before = "fn main() {\n    old();\n}\n";
    let after = "fn main() {\n    new();\n}\n";
    state.event(Event::ToolStarted {
        id: "edit".into(),
        name: "edit".into(),
        arguments: Value::object([("path", Value::string("fixture.rs"))]),
    });
    let change = crate::file_change::Change::between(Some(before), after).unwrap();
    state.event(Event::ToolChanged {
        id: "edit".into(),
        change: change.clone(),
    });
    state.event(Event::ToolFinished {
        id: "edit".into(),
        name: "edit".into(),
        summary: "updated".into(),
        result: Value::object([]),
    });
    let original = state.rows()[0].clone();
    let text = original
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("● Update(fixture.rs)"));
    assert!(text.contains("⎿ +1 −1 lines"));
    assert!(text.contains("old();") && text.contains("new();"));
    start(&mut state, "next");
    assert_eq!(state.rows()[0], original);
    assert!(matches!(&state.items[0], Item::Tool { change: Some(saved), .. } if **saved == change));
    assert_eq!(state.rows()[1][0], Line::default());
    assert!(
        state.rows()[1][1]
            .plain()
            .starts_with("● Bash(fixture-command)")
    );
    assert!(
        state.rows()[1]
            .iter()
            .any(|row| row.plain().contains("Bash(fixture-command)"))
    );
}

#[test]
fn reused_tool_id_keeps_each_captured_diff_with_its_own_call() {
    let mut state = State::default();
    for (before, after) in [("one\n", "two\n"), ("two\n", "three\n")] {
        state.event(Event::ToolStarted {
            id: "reused".into(),
            name: "edit".into(),
            arguments: Value::object([("path", Value::string("fixture.txt"))]),
        });
        state.event(Event::ToolChanged {
            id: "reused".into(),
            change: crate::file_change::Change::between(Some(before), after).unwrap(),
        });
        state.event(Event::ToolFinished {
            id: "reused".into(),
            name: "edit".into(),
            summary: "updated".into(),
            result: Value::object([]),
        });
    }
    let rows = state.rows();
    assert_eq!(rows.len(), 2);
    let first = rows[0]
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    let second = rows[1]
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(first.contains("one") && first.contains("two"));
    assert!(!first.contains("three"));
    assert!(second.contains("two") && second.contains("three"));
    assert!(!second.contains("one"));
    assert_eq!(rows[1][0], Line::default());
}

#[test]
fn a_notice_settles_tool_bookkeeping_without_losing_its_pending_result() {
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
    assert_ne!(state.rows()[..1], settled);
    assert!(visible(&state).contains("2 calls"));
    assert!(visible(&state).contains("1 command") && visible(&state).contains("1 running"));
    assert!(
        matches!(&state.items[0], Item::Tool { result: Some(result), .. }
        if result.get("stdout").and_then(Value::as_str) == Some("retained result"))
    );
}
