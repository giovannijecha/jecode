use super::*;
use crate::{events::Event, json, json::Value, tui::cards};
use std::time::Duration;

fn call(state: &mut State, id: &str, name: &str, arguments: &str, result: &str) {
    state.event(Event::ToolStarted {
        id: id.into(),
        name: name.into(),
        arguments: json::parse(arguments).unwrap(),
    });
    let result = json::parse(result).unwrap();
    state.event(Event::ToolFinished {
        id: id.into(),
        name: name.into(),
        summary: crate::events::tool_summary(name, &result),
        result,
    });
}

fn complete(state: &State) -> Layout {
    let mut layout = Layout::default();
    for _ in 0..100 {
        layout.prepare(state, state.width, 64);
        layout.prepare_pending(state, 4096);
        if layout.ready(state) && layout.pending_ready(state) {
            return layout;
        }
    }
    panic!("the test turn did not settle");
}

fn finished_turn(columns: usize) -> State {
    let mut state = State {
        width: columns,
        ..State::default()
    };
    state.begin_tool_turn();
    call(
        &mut state,
        "todo",
        "todo_list",
        r#"{"action":"set","title":"Verification","tasks":[{"id":"verify","title":"Verify the complete turn","status":"pending","note":"Complete task note sentinel"}]}"#,
        r#"{"title":"Verification","tasks":[{"id":"verify","title":"Verify the complete turn","status":"pending","note":"Complete task note sentinel"}]}"#,
    );
    call(
        &mut state,
        "ask",
        "ask_user",
        r#"{"questions":[{"id":"rows","title":"Rows","question":"Which rows should be included?","context":"Question context sentinel","options":[{"id":"all","label":"All rows","description":"Complete option description sentinel"}]}]}"#,
        r#"{"outcome":"answered","answers":[{"id":"rows","choices":["all"],"text":"Answer sentinel"}]}"#,
    );
    for id in ["first", "second"] {
        call(
            &mut state,
            id,
            "read",
            &format!(r#"{{"path":"{id}.rs","work":"Inspect parser"}}"#),
            r#"{"content":"1: captured source\n2: final source sentinel","offset":1,"total_lines":2}"#,
        );
    }
    state.message(Kind::Assistant, "The following cards are outside WORK.");
    call(
        &mut state,
        "write",
        "write",
        r#"{"path":"notes.md","content":"Updated notes"}"#,
        r#"{"bytes_written":13}"#,
    );
    call(
        &mut state,
        "verify",
        "bash",
        r#"{"description":"Run the independent verification command","command":"verify --all"}"#,
        r#"{"exit_code":0,"stdout":"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nfinal output sentinel","stderr":""}"#,
    );
    state.close_tools();
    state.finish_tool_turn();
    for item in &mut state.items {
        if let Item::Tool { presentation, .. } = item {
            presentation.started = None;
            presentation.elapsed = Some(Duration::ZERO);
        }
    }
    state
}

#[test]
fn the_work_footer_owns_expand_and_the_first_card_owns_collapse_for_the_whole_turn() {
    for columns in [40, 80, 120] {
        let mut state = finished_turn(columns);
        assert_eq!(keycap(&state, columns), Some(2));
        let compact = complete(&state);
        assert!(
            compact.blocks[3]
                .last()
                .unwrap()
                .plain()
                .contains("Ctrl+O expand")
        );
        assert_eq!(
            compact
                .blocks
                .iter()
                .flatten()
                .filter(|row| row.plain().contains("Ctrl+O"))
                .count(),
            1
        );

        assert_eq!(state.toggle_tools(), Some((true, 6)));
        assert_eq!(keycap(&state, columns), Some(0));
        let expanded = complete(&state);
        assert_eq!(
            expanded
                .blocks
                .iter()
                .flatten()
                .filter(|row| row.plain().contains("Ctrl+O collapse"))
                .count(),
            1
        );
        assert!(
            !expanded
                .blocks
                .iter()
                .flatten()
                .any(|row| row.plain().starts_with("  WORK  "))
        );
        for (index, item) in state.items.iter().enumerate() {
            let Some(tool) = item.tool() else { continue };
            let block = &expanded.blocks[index + 1];
            let shown = if block.first().is_some_and(|row| row.plain().is_empty()) {
                &block[1..]
            } else {
                block.as_slice()
            };
            assert_eq!(
                shown,
                cards::card(tool, columns, index == 0).rows,
                "tool {index} at {columns} columns differs from its standalone card"
            );
        }
        let text = expanded
            .blocks
            .iter()
            .flatten()
            .map(Line::plain)
            .collect::<Vec<_>>()
            .join("\n")
            .replace(['│', '└'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        for sentinel in [
            "task note sentinel",
            "context sentinel",
            "description sentinel",
            "Answer sentinel",
            "source sentinel",
            "output sentinel",
        ] {
            assert!(
                text.contains(sentinel),
                "missing full {sentinel} at {columns} columns: {text}"
            );
        }
        assert_eq!(state.toggle_tools(), Some((false, 6)));
        assert_eq!(keycap(&state, columns), Some(2));
    }
}

#[test]
fn a_single_read_owns_expand_even_without_a_compact_body() {
    let mut state = State::default();
    state.begin_tool_turn();
    call(
        &mut state,
        "read",
        "read",
        r#"{"path":"source.rs"}"#,
        r#"{"content":"captured source"}"#,
    );
    state.close_tools();
    state.finish_tool_turn();
    let layout = complete(&state);
    assert_eq!(keycap(&state, state.width), Some(0));
    assert!(
        layout.blocks[1]
            .iter()
            .any(|line| line.plain().contains("Ctrl+O expand"))
    );
}

#[test]
fn standalone_question_and_todo_cards_expose_their_full_content_key() {
    let cases = [
        (
            "todo_list",
            r#"{"action":"set","title":"Verification","tasks":[{"id":"verify","title":"Check all details","status":"pending","note":"A hidden task note"}]}"#,
            r#"{"title":"Verification","tasks":[{"id":"verify","title":"Check all details","status":"pending","note":"A hidden task note"}]}"#,
        ),
        (
            "ask_user",
            r#"{"questions":[{"id":"rows","title":"Rows","question":"Which rows?","context":"A hidden question context","options":[{"id":"all","label":"All rows"}]}]}"#,
            r#"{"outcome":"answered","answers":[{"id":"rows","choices":["all"],"text":""}]}"#,
        ),
    ];
    for columns in [40, 80, 120] {
        for (name, arguments, result) in cases {
            let mut state = State {
                width: columns,
                ..State::default()
            };
            state.begin_tool_turn();
            call(&mut state, "single", name, arguments, result);
            state.close_tools();
            state.finish_tool_turn();
            assert_eq!(keycap(&state, columns), Some(0), "{name}");
            let compact = complete(&state);
            assert_eq!(
                compact
                    .blocks
                    .iter()
                    .flatten()
                    .filter(|row| row.plain().contains("Ctrl+O expand"))
                    .count(),
                1,
                "{name}"
            );
            assert_eq!(state.toggle_tools(), Some((true, 1)));
            let expanded = complete(&state);
            assert_eq!(
                expanded
                    .blocks
                    .iter()
                    .flatten()
                    .filter(|row| row.plain().contains("Ctrl+O collapse"))
                    .count(),
                1,
                "{name}"
            );
        }
    }
}

#[test]
fn an_unresolved_hidden_question_does_not_take_collapse_from_the_first_visible_card() {
    let mut state = State::default();
    state.begin_tool_turn();
    state.event(Event::ToolStarted {
        id: "waiting".into(),
        name: "ask_user".into(),
        arguments: Value::object([]),
    });
    call(
        &mut state,
        "read",
        "read",
        r#"{"path":"source.rs"}"#,
        r#"{"content":"captured source"}"#,
    );
    state.close_tools();
    state.finish_tool_turn();
    assert_eq!(state.toggle_tools(), Some((true, 2)));
    assert_eq!(keycap(&state, state.width), Some(1));
    let layout = complete(&state);
    assert_eq!(
        layout
            .blocks
            .iter()
            .flatten()
            .chain(layout.pending_blocks().iter().flatten())
            .filter(|row| row.plain().contains("Ctrl+O collapse"))
            .count(),
        1
    );
}
