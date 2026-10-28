use super::*;
use crate::tui::theme::{
    BADGE_BAD, BADGE_IDLE, BADGE_TEXT, BADGE_WARN, LIT, S_BASE, STRONG, Style, Surface,
};
use std::time::Duration;

fn object<const N: usize>(fields: [(&str, &str); N]) -> Value {
    Value::object(fields.map(|(key, value)| (key, Value::string(value))))
}

fn tool<'a>(
    name: &'a str,
    arguments: &'a Value,
    result: Option<&'a Value>,
    presentation: &'a Presentation,
) -> Tool<'a> {
    Tool {
        name,
        arguments,
        summary: "complete",
        result,
        change: None,
        presentation,
    }
}

/// The WORK badge's name span: its ink and surface.
fn badge(card: &Card) -> (Style, Option<Surface>) {
    let name = card.rows[0]
        .spans
        .iter()
        .find(|span| span.text.contains("WORK"))
        .unwrap();
    (name.style, name.background)
}

fn plain(card: &Card) -> String {
    card.rows
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn labels_are_trimmed_and_skill_reads_stay_outside_chapters() {
    let shown = Presentation::default();
    let read = object([("path", "src/lib.rs"), ("work", " Inspect source ")]);
    assert_eq!(
        chapter::label(&tool("read", &read, None, &shown)),
        "Inspect source"
    );
    for work in ["", "  \t "] {
        let args = object([("path", "src/lib.rs"), ("work", work)]);
        assert_eq!(
            chapter::label(&tool("read", &args, None, &shown)),
            "Activity"
        );
    }
    let missing = object([("path", "src/lib.rs")]);
    assert_eq!(
        chapter::label(&tool("read", &missing, None, &shown)),
        "Activity"
    );
    for name in [
        "read", "list", "search", "bash", "exec", "process", "web", "browser",
    ] {
        assert!(chapter::groupable(&tool(name, &missing, None, &shown)));
    }
    let skill = object([("path", "skill:review/SKILL.md")]);
    assert!(!chapter::groupable(&tool("read", &skill, None, &shown)));
    for name in ["write", "edit", "checkpoint"] {
        assert!(!chapter::groupable(&tool(name, &missing, None, &shown)));
    }
}

#[test]
fn discovery_calls_join_work_chapters_and_count_each_kind() {
    let shown = Presentation::default();
    let args = object([("path", "src"), ("work", "Locate implementation")]);
    let listing = Value::object([("entries_returned", Value::number(2))]);
    let search = Value::object([("matches_returned", Value::number(1))]);
    let calls = [
        tool("list", &args, Some(&listing), &shown),
        tool("search", &args, Some(&search), &shown),
    ];
    let text = plain(&chapter::header(&calls, 80, false));
    assert!(text.contains("Locate implementation"), "{text}");
    assert!(text.contains("1 listing"), "{text}");
    assert!(text.contains("1 search"), "{text}");
    let partial = Value::object([
        ("entries_returned", Value::number(2)),
        ("errors_count", Value::number(1)),
        ("complete", Value::Bool(false)),
    ]);
    let warning = chapter::exception(&tool("list", &args, Some(&partial), &shown), 80);
    assert!(warning[0].plain().contains("1 error"));
}

#[test]
fn exec_joins_work_chapters_and_counts_as_a_command() {
    let shown = Presentation::default();
    let args = Value::object([
        ("executable", Value::string("cargo")),
        ("args", Value::Array(vec![Value::string("test")])),
        ("work", Value::string("Verify")),
        ("description", Value::string("Run tests")),
    ]);
    let result = Value::object([
        ("exit_code", Value::number(0)),
        ("stdout", Value::string("ok\n")),
    ]);
    let calls = [
        tool("exec", &args, Some(&result), &shown),
        tool("exec", &args, Some(&result), &shown),
    ];
    let header = chapter::header(&calls, 80, false);
    assert!(plain(&header).contains("2 commands"));
    assert!(plain(&header).contains("Verify"));
    let exception = chapter::exception(&calls[0], 80);
    assert!(exception[0].plain().contains("EXEC Run tests"));
    let receipts = chapter::receipts(&calls[0], 80);
    assert!(
        receipts
            .iter()
            .any(|line| line.plain().contains("Command: cargo [\"test\"]"))
    );
}

#[test]
fn displayed_work_label_is_one_line_and_bounded_without_changing_arguments() {
    let shown = Presentation::default();
    for work in [
        "  Inspect\n\t parser\r\n now  ".to_string(),
        "界👩‍💻e\u{301} ".repeat(100),
    ] {
        let args = object([("path", "src/lib.rs"), ("work", &work)]);
        let calls = [
            tool("read", &args, None, &shown),
            tool("read", &args, None, &shown),
        ];
        let card = chapter::header(&calls, 200, false);
        let title = card
            .rows
            .iter()
            .filter(|row| row.origin.is_some_and(|origin| origin.line == 0))
            .map(Line::plain)
            .collect::<Vec<_>>();
        assert_eq!(title.len(), 1, "{title:?}");
        let label = title[0]
            .strip_prefix(" WORK  ")
            .unwrap()
            .split("  2 calls")
            .next()
            .unwrap();
        assert!(text::cells(label) <= 60, "{label}");
        assert!(!label.contains(['\n', '\r', '\t']));
        assert_eq!(
            args.get("work").and_then(Value::as_str),
            Some(work.as_str())
        );
        if work.contains("parser") {
            assert_eq!(label, "Inspect parser now");
        } else {
            assert!(label.ends_with('…'));
        }
    }
}

#[test]
fn header_counts_unique_files_and_completed_routine_actions() {
    let shown = Presentation::default();
    let args = [
        object([("path", "src/lib.rs")]),
        object([("path", "src/lib.rs")]),
        object([("path", "tmp:notes.txt")]),
        object([("path", "history:requests")]),
        object([("action", "fetch")]),
        object([("action", "screenshot")]),
        object([("action", "snapshot")]),
        object([("action", "click")]),
        object([("command", "cargo test")]),
        object([("action", "poll")]),
    ];
    let ok = Value::object([]);
    let screenshot = object([("screenshot_path", "tmp:browser/shot.png")]);
    let snapshot = object([("snapshot_path", "tmp:browser/snapshot.txt")]);
    let values = [
        &ok,
        &ok,
        &ok,
        &ok,
        &ok,
        &screenshot,
        &snapshot,
        &ok,
        &ok,
        &ok,
    ];
    let names = [
        "read", "read", "read", "read", "web", "browser", "browser", "browser", "bash", "process",
    ];
    let tools: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(index, name)| tool(name, &args[index], Some(values[index]), &shown))
        .collect();
    let card = chapter::header(&tools, 200, false);
    let text = plain(&card);
    for expected in [
        "10 calls",
        "2 files read",
        "1 other read",
        "1 web call",
        "1 screenshot",
        "1 snapshot",
        "1 browser action",
        "1 command",
        "1 process call",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert!(!text.contains("What happened") && !text.contains("10 done"));
    assert!(card.hidden);
    assert!(
        card.rows
            .iter()
            .all(|row| row.origin.is_some_and(|origin| origin.line <= 1))
    );
}

#[test]
fn status_priority_counts_every_outcome_without_summing_durations() {
    let args = object([("command", "cargo test"), ("work", "Verify")]);
    let shown = Presentation {
        elapsed: Some(Duration::from_secs(6)),
        ..Presentation::default()
    };
    let failed = object([("error", "failed")]);
    let background = object([("process_id", "p1"), ("status", "running")]);
    let stopped = Value::object([("cancelled", Value::Bool(true))]);
    let done = Value::object([("exit_code", Value::number(0))]);
    let tools = [
        tool("bash", &args, Some(&failed), &shown),
        tool("bash", &args, None, &shown),
        tool("bash", &args, Some(&background), &shown),
        tool("bash", &args, Some(&stopped), &shown),
        tool("bash", &args, Some(&done), &shown),
    ];
    let card = chapter::header(&tools, 200, false);
    let text = plain(&card);
    assert_eq!(badge(&card), (BADGE_TEXT.bold(), Some(BADGE_BAD)));
    assert!(text.contains("5 calls · 1 failed"));
    assert!(text.contains("1 command · 1 stopped · 1 running · 1 in background"));
    assert!(!card.rows[1].plain().contains("failed"));
    assert!(!text.contains("6s") && !text.contains('▶'));

    let running = chapter::header(&tools[1..], 200, false);
    // Running shimmers across the name on the quiet surface; frame 0 rests.
    assert_eq!(badge(&running), (S_BASE.bold(), Some(BADGE_IDLE)));
    assert_eq!(running.rows[0].spans[1].text, "WORK");
    let background = chapter::header(&tools[2..], 200, false);
    assert_eq!(badge(&background), (LIT.bold(), Some(BADGE_IDLE)));
    let stopped = chapter::header(&tools[3..], 200, false);
    assert_eq!(badge(&stopped), (BADGE_TEXT.bold(), Some(BADGE_WARN)));
    let done = chapter::header(&tools[4..], 200, false);
    assert_eq!(badge(&done), (STRONG.bold(), Some(BADGE_IDLE)));

    let cancelled_exit = Value::object([
        ("cancelled", Value::Bool(true)),
        ("exit_code", Value::number(101)),
    ]);
    let stopped = chapter::header(
        &[tool("bash", &args, Some(&cancelled_exit), &shown)],
        80,
        false,
    );
    assert_eq!(badge(&stopped), (BADGE_TEXT.bold(), Some(BADGE_WARN)));
    assert!(plain(&stopped).contains("stopped"));
    assert_eq!(stopped.rows.len(), 2);
    // The header leaves its tree open for the chapter's later rows.
    assert_eq!(stopped.rows[1].plain(), "  │ 1 stopped");
}

#[test]
fn unicode_titles_wrap_under_two_cells_and_keycap_only_hints_hidden_calls() {
    let args = object([
        ("path", "src/界面/🦀.rs"),
        ("work", "Inspect 界面 🦀 layout"),
    ]);
    let result = Value::object([]);
    let collapsed = Presentation::default();
    let expanded = Presentation {
        expanded: true,
        ..Presentation::default()
    };
    for columns in [12, 20, 40] {
        let call = tool("read", &args, Some(&result), &collapsed);
        let card = chapter::header(&[call], columns, true);
        assert!(card.hidden);
        assert_eq!(card.rows.len(), 2);
        assert!(card.rows[0].plain().contains("Ctrl+O"));
        assert!(!card.rows[1].plain().contains("Ctrl+O"));
        assert!(card.rows[0].plain().trim_end().ends_with(if columns >= 19 {
            "expand"
        } else {
            "Ctrl+O"
        }));
        assert!(
            card.rows
                .iter()
                .all(|row| text::cells(&row.plain()) <= columns)
        );
        assert!(
            card.rows
                .iter()
                .skip(1)
                .filter(|row| row.origin.is_some_and(|origin| origin.line == 0))
                .all(|row| row.plain().starts_with("  "))
        );

        let open = chapter::header(
            &[tool("read", &args, Some(&result), &expanded)],
            columns,
            true,
        );
        // Expanded, the keycap names the way back instead of hidden calls.
        assert!(!open.hidden);
        assert!(open.rows[0].plain().contains("Ctrl+O"));
        assert!(!plain(&open).contains("expand"));
        assert!(
            open.rows[1..]
                .iter()
                .all(|row| !row.plain().contains("Ctrl+O"))
        );
        assert!(open.rows[0].plain().trim_end().ends_with(if columns >= 19 {
            "collapse"
        } else {
            "Ctrl+O"
        }));
        assert!(
            open.rows
                .iter()
                .all(|row| text::cells(&row.plain()) <= columns)
        );
    }
    let pending = chapter::header(&[tool("read", &args, None, &collapsed)], 40, true);
    assert!(!pending.hidden);
    assert!(!plain(&pending).contains("Ctrl+O"));
}

#[test]
fn visible_exceptions_have_target_and_result_without_a_second_dot() {
    let args = object([("command", "cargo test"), ("work", "Verify")]);
    let shown = Presentation::default();
    let failed = Value::object([
        ("exit_code", Value::number(101)),
        ("stderr", Value::string("missing import\n")),
    ]);
    let stopped = Value::object([("cancelled", Value::Bool(true))]);
    let background = object([("process_id", "p1"), ("status", "running")]);
    for (result, expected) in [
        (Some(&failed), "exit 101"),
        (Some(&stopped), "stopped"),
        (Some(&background), "background"),
        (None, "running"),
    ] {
        let rows = chapter::exception(&tool("bash", &args, result, &shown), 40);
        let text = rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n");
        assert!(text.starts_with("  ⎿ BASH cargo test"), "{text}");
        assert!(text.contains(expected), "{text}");
        assert_eq!(rows.len(), 1, "{text}");
        assert!(!text.contains("p1") && !text.contains("missing import"));
        assert!(!text.contains('●') && !text.contains('▶'));
        assert_eq!(rows[0].spans[1].style, LIT.bold());
        assert_eq!(rows[0].spans[2].style, crate::tui::theme::BODY);
        assert!(rows.iter().all(|row| text::cells(&row.plain()) <= 40));
    }
    let ok = Value::object([("exit_code", Value::number(0))]);
    let rows = chapter::exception(&tool("bash", &args, Some(&ok), &shown), 40);
    assert_eq!(rows[0].plain(), "  ⎿ BASH cargo test");
    assert_eq!(rows.len(), 1);
    assert!(chapter::exception(&tool("read", &args, Some(&ok), &shown), 40).is_empty());
}

#[test]
fn compact_process_hides_streams_and_activity_skips_a_visible_failure() {
    let args = object([("action", "poll"), ("process_id", "private-pid")]);
    let result = Value::object([
        ("process_id", Value::string("private-pid")),
        ("status", Value::string("running")),
        ("stdout", Value::string("raw process output")),
    ]);
    let shown = Presentation::default();
    let rows = chapter::exception(&tool("process", &args, Some(&result), &shown), 80);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].plain(), "  ⎿ PROCESS poll  background");
    assert!(!rows[0].plain().contains("private-pid"));
    assert!(!rows[0].plain().contains("raw process output"));

    let first = object([("path", "src/good.rs")]);
    let last = object([("path", "src/missing.rs")]);
    let ok = Value::object([]);
    let failed = object([("error", "permission denied")]);
    let calls = [
        tool("read", &first, Some(&ok), &shown),
        tool("read", &last, Some(&failed), &shown),
    ];
    let activity = chapter::activity(&calls, 80).unwrap().plain();
    assert!(activity.contains("READ src/good.rs"), "{activity}");
    assert!(!activity.contains("src/missing.rs"), "{activity}");
    assert!(!activity.contains("failed"), "{activity}");
    assert!(chapter::activity(&calls[1..], 80).is_none());
}

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

#[test]
fn facts_omit_repeated_uniform_states_but_keep_mixed_outcomes() {
    let shown = Presentation::default();
    let args = object([("command", "cargo test"), ("work", "Verify")]);
    let failed = object([("error", "failed")]);
    let stopped = Value::object([("cancelled", Value::Bool(true))]);
    let calls = [tool("bash", &args, Some(&failed), &shown); 2];
    let card = chapter::header(&calls, 80, false);
    assert_eq!(
        plain(&card),
        " WORK  Verify  2 calls · 2 failed\n  │ 0 completed"
    );
    let mixed = chapter::header(
        &[calls[0], tool("bash", &args, Some(&stopped), &shown)],
        80,
        false,
    );
    assert_eq!(
        plain(&mixed),
        " WORK  Verify  2 calls · 1 failed\n  │ 1 stopped"
    );
}

#[test]
fn compact_commands_keep_notices_as_receipts_when_content_height_needs_them() {
    let args = object([("command", "cargo test")]);
    let shown = Presentation::default();
    for status in ["partial", "none"] {
        let result = Value::object([
            ("exit_code", Value::number(0)),
            (
                "checkpoint",
                object([("status", status), ("reason", "Untracked files")]),
            ),
            ("stdout_truncated", Value::Bool(true)),
            ("stdout_bytes", Value::number(2048)),
            ("output_limit_bytes", Value::number(1024)),
        ]);
        let call = tool("bash", &args, Some(&result), &shown);
        let rows = chapter::exception(&call, 80);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].plain(), "  ⎿ BASH cargo test");
        let receipts = chapter::receipts(&call, 80);
        let text = receipts
            .iter()
            .map(Line::plain)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            text.contains(&format!("Checkpoint {status} · Untracked files")),
            status == "partial",
            "{text}"
        );
        assert!(
            text.contains("stdout capture kept 1 KiB of 2 KiB"),
            "{text}"
        );
        assert!(text.contains("exit 0"), "{text}");
    }
}

#[test]
fn long_compact_command_targets_keep_a_visible_ellipsis_within_two_rows() {
    let args = object([(
        "command",
        &format!("printf {} tail", "界example ".repeat(80)),
    )]);
    let shown = Presentation::default();
    let result = Value::object([("exit_code", Value::number(0))]);
    for columns in [20, 40, 80] {
        let rows = chapter::exception(&tool("bash", &args, Some(&result), &shown), columns);
        assert!(rows.len() <= 2);
        assert!(rows.last().unwrap().plain().ends_with('…'));
        assert!(rows.iter().all(|row| text::cells(&row.plain()) <= columns));
    }
}
