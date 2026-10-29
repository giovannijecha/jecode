use super::*;

#[test]
fn running_readlike_members_keep_each_target_until_their_own_result_arrives() {
    let shown = Presentation {
        elapsed: Some(Duration::from_secs(2)),
        ..Presentation::default()
    };
    let first = object([("path", "src/first.rs")]);
    let second = object([("path", "src/second.rs")]);
    let completed = Value::object([]);
    for columns in [40, 80] {
        let pending = [
            tool("read", &first, None, &shown),
            tool("read", &second, None, &shown),
        ];
        let rows = pending
            .iter()
            .flat_map(|call| chapter::exception(call, columns))
            .map(|row| row.plain())
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        assert!(
            rows[0].contains("first.rs") && rows[1].contains("second.rs"),
            "{rows:?}"
        );
        assert!(rows.iter().all(|row| text::cells(row) <= columns));
        assert!(chapter::activity(&pending, columns).is_none());

        let second_done = [pending[0], tool("read", &second, Some(&completed), &shown)];
        assert_eq!(chapter::exception(&second_done[0], columns).len(), 1);
        assert!(chapter::exception(&second_done[1], columns).is_empty());
        let foot = chapter::activity(&second_done, columns).unwrap().plain();
        assert!(
            foot.contains("second.rs") && !foot.contains("first.rs"),
            "{foot}"
        );
    }
}

#[test]
fn completed_activity_rows_make_room_for_running_targets_without_growing_unbounded() {
    let shown = Presentation::default();
    let args: Vec<_> = (0..5)
        .map(|index| object([("path", &format!("src/{index}.rs"))]))
        .collect();
    let completed = Value::object([]);
    let mut calls: Vec<_> = args[..4]
        .iter()
        .map(|args| tool("read", args, Some(&completed), &shown))
        .collect();
    let before = chapter::activities(&calls, 80);
    assert_eq!(before.len(), 4);
    calls.push(tool("read", &args[4], None, &shown));
    let running = chapter::activities(&calls, 80);
    assert_eq!(running.len(), 3);
    assert!(running.last().unwrap().plain().contains("3.rs"));
    calls[4] = tool("read", &args[4], Some(&completed), &shown);
    let after = chapter::activities(&calls, 80);
    assert_eq!(after.len(), 4);
    assert!(after.last().unwrap().plain().contains("4.rs"));
    assert!(!after.iter().any(|row| row.plain().contains("0.rs")));
}

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
        assert!(chapter[0].plain().contains("Ctrl+O"));
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
