use super::*;

#[test]
fn out_of_order_completions_update_source_order_and_keep_turn_expansion() {
    use crate::{events::Event, tui::state::State};
    for columns in [40, 80] {
        let mut state = State {
            width: columns,
            ..State::default()
        };
        state.begin_tool_turn();
        for (id, path) in [("a", "src/a.rs"), ("b", "src/b.rs"), ("c", "src/c.rs")] {
            state.event(Event::ToolStarted {
                id: id.into(),
                name: "read".into(),
                arguments: Value::object([
                    ("path", Value::string(path)),
                    ("work", Value::string("Inspect sources")),
                ]),
            });
        }
        let visible = |state: &State| {
            state
                .rows()
                .into_iter()
                .flatten()
                .map(|row| row.plain())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let started = visible(&state);
        assert!(started.contains("3 running"), "{started}");
        for path in ["a.rs", "b.rs", "c.rs"] {
            assert!(started.contains(path), "{started}");
        }
        for (id, remaining) in [("b", 2), ("a", 1), ("c", 0)] {
            state.event(Event::ToolFinished {
                id: id.into(),
                name: "read".into(),
                summary: "1 line".into(),
                result: Value::object([("lines_returned", Value::number(1))]),
            });
            let snapshot = visible(&state);
            if remaining > 0 {
                assert!(
                    snapshot.contains(&format!("{remaining} running")),
                    "{snapshot}"
                );
            } else {
                assert!(!snapshot.contains("running"), "{snapshot}");
            }
            assert_eq!(
                state
                    .items
                    .iter()
                    .filter(|item| matches!(
                        item,
                        crate::tui::state::Item::Tool { result: None, .. }
                    ))
                    .count(),
                remaining
            );
            assert!(
                state
                    .rows()
                    .into_iter()
                    .flatten()
                    .all(|row| text::cells(&row.plain()) <= columns)
            );
        }
        state.close_tools();
        state.finish_tool_turn();
        assert_eq!(state.toggle_tools(), Some((true, 3)));
        let expanded = visible(&state);
        for path in ["a.rs", "b.rs", "c.rs"] {
            assert!(expanded.contains(path), "{expanded}");
        }
        let mut rows = crate::tui::state::rows::Rows::default();
        let chapter = rows.render_at(&state.items, 0, columns, true);
        assert!(
            chapter.iter().any(|row| row.plain().contains("Ctrl+O")),
            "{chapter:?}"
        );
    }
}

#[test]
fn out_of_order_edit_diffs_stay_with_their_standalone_cards() {
    use crate::{events::Event, file_change::Change, tui::state::State};
    let mut state = State {
        width: 80,
        ..State::default()
    };
    state.begin_tool_turn();
    for (id, path) in [("first", "src/first.rs"), ("second", "src/second.rs")] {
        state.event(Event::ToolStarted {
            id: id.into(),
            name: "edit".into(),
            arguments: Value::object([("path", Value::string(path))]),
        });
    }
    for (id, before, after) in [
        ("second", "let second = 1;\n", "let second = 2;\n"),
        ("first", "let first = 1;\n", "let first = 2;\n"),
    ] {
        state.event(Event::ToolChanged {
            id: id.into(),
            change: Change::between(Some(before), after).unwrap(),
        });
        state.event(Event::ToolFinished {
            id: id.into(),
            name: "edit".into(),
            summary: "updated".into(),
            result: Value::object([("bytes_written", Value::number(after.len()))]),
        });
    }
    let blocks = state.rows();
    assert_eq!(blocks.len(), 2);
    let first = blocks[0]
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    let second = blocks[1]
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        first.contains("first.rs") && first.contains("first = 2"),
        "{first}"
    );
    assert!(!first.contains("second = 2"), "{first}");
    assert!(
        second.contains("second.rs") && second.contains("second = 2"),
        "{second}"
    );
    assert!(!second.contains("first = 2"), "{second}");
}
