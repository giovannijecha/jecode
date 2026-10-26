use super::*;
use crate::tui::{cards, line::Line};

fn command(state: &mut State, id: &str, command: &str, work: Option<&str>) {
    let mut arguments = Value::object([("command", Value::string(command))]);
    if let Some(work) = work
        && let Value::Object(fields) = &mut arguments
    {
        fields.insert("work".into(), Value::string(work));
    }
    state.event(Event::ToolStarted {
        id: id.into(),
        name: "bash".into(),
        arguments,
    });
    state.event(Event::ToolFinished {
        id: id.into(),
        name: "bash".into(),
        summary: "preview output".into(),
        result: Value::object([
            ("exit_code", Value::number(0)),
            ("stdout", Value::string("preview output\n")),
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
fn singleton_command_keeps_its_existing_card_preview_in_older_turns() {
    let mut state = State::default();
    state.begin_tool_turn();
    command(
        &mut state,
        "one",
        "printf actual-command",
        Some("Read docs"),
    );
    state.close_tools();
    state.finish_tool_turn();
    let before = visible(&state);
    assert!(before.contains("● Bash(printf actual-command)"), "{before}");
    assert!(before.contains("preview output"), "{before}");
    assert!(!before.contains("Read docs") && !before.contains("1 call"));
    state.begin_tool_turn();
    command(&mut state, "next", "printf another-command", None);
    state.close_tools();
    state.finish_tool_turn();
    assert!(visible(&state).starts_with(&before));
    assert_eq!(state.toggle_tools(), Some((true, 1)));
    assert!(visible(&state).starts_with(&before));
}

#[test]
fn every_completed_bash_target_stays_visible_below_a_label_and_in_older_turns() {
    for work in [None, Some("Read docs")] {
        let mut state = State::default();
        state.begin_tool_turn();
        for (id, cmd) in [
            ("first", "printf first-command"),
            ("second", "printf second-command"),
        ] {
            command(&mut state, id, cmd, work);
        }
        state.close_tools();
        state.finish_tool_turn();
        let before = visible(&state);
        assert!(before.contains("(2 calls)"), "{before}");
        for cmd in ["printf first-command", "printf second-command"] {
            assert!(before.contains(cmd), "{before}");
        }
        state.begin_tool_turn();
        command(&mut state, "next", "printf next", None);
        state.close_tools();
        state.finish_tool_turn();
        assert!(visible(&state).starts_with(&before));
        state.toggle_tools();
        assert!(visible(&state).starts_with(&before));
    }
}

#[test]
fn every_routine_singleton_uses_the_existing_card_renderer() {
    for (name, arguments) in [
        (
            "read",
            Value::object([("path", Value::string("src/lib.rs"))]),
        ),
        (
            "bash",
            Value::object([("command", Value::string("printf hello"))]),
        ),
        (
            "process",
            Value::object([("action", Value::string("list"))]),
        ),
        (
            "web",
            Value::object([
                ("action", Value::string("fetch")),
                ("url", Value::string("https://example.org")),
            ]),
        ),
        (
            "browser",
            Value::object([("action", Value::string("snapshot"))]),
        ),
    ] {
        let mut state = State {
            width: 80,
            ..State::default()
        };
        state.begin_tool_turn();
        state.event(Event::ToolStarted {
            id: name.into(),
            name: name.into(),
            arguments,
        });
        for result in [None, Some(Value::object([]))] {
            if let Some(result) = result {
                state.event(Event::ToolFinished {
                    id: name.into(),
                    name: name.into(),
                    summary: "Done".into(),
                    result,
                });
            }
            let expected = cards::card(state.items[0].tool().unwrap(), 80, false);
            assert_eq!(state.rows()[0], expected.rows, "{name}");
        }
    }
}

#[test]
fn a_second_matching_call_converts_the_singleton_and_invalidates_its_cached_preview() {
    use crate::tui::transcript::Layout;
    let mut state = State::default();
    state.begin_tool_turn();
    command(&mut state, "one", "printf first", Some("Inspect"));
    state.close_tools();
    let mut layout = Layout::default();
    layout.prepare(&state, 80, 64);
    assert!(
        layout
            .blocks
            .iter()
            .flatten()
            .any(|row| row.plain().contains("Bash(printf first)"))
    );
    command(&mut state, "two", "printf second", Some("Inspect"));
    state.close_tools();
    layout.prepare(&state, 80, 64);
    let rows = layout
        .blocks
        .iter()
        .flatten()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rows.contains("● Inspect (2 calls)"), "{rows}");
    assert!(
        rows.contains("Bash printf first") && rows.contains("Bash printf second"),
        "{rows}"
    );
    assert!(
        !rows.contains("Bash(printf first)") && !rows.contains("preview output"),
        "{rows}"
    );
}
