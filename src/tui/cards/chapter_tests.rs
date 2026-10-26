use super::*;
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
    for name in ["read", "bash", "process", "web", "browser"] {
        assert!(chapter::groupable(&tool(name, &missing, None, &shown)));
    }
    let skill = object([("path", "skill:review/SKILL.md")]);
    assert!(!chapter::groupable(&tool("read", &skill, None, &shown)));
    for name in ["write", "edit", "checkpoint"] {
        assert!(!chapter::groupable(&tool(name, &missing, None, &shown)));
    }
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
            .strip_prefix("● ")
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
    assert_eq!(card.rows[0].spans[0].style, BAD);
    assert!(text.contains("5 calls · 1 failed"));
    assert!(text.contains("1 command · 1 stopped · 1 running · 1 in background"));
    assert!(!card.rows[1].plain().contains("failed"));
    assert!(!text.contains("6s") && !text.contains('▶'));

    let running = chapter::header(&tools[1..], 200, false);
    assert_eq!(running.rows[0].spans[0].style, TOOL_PULSE[1]);
    let background = chapter::header(&tools[2..], 200, false);
    assert_eq!(background.rows[0].spans[0].style, TOOL_ACCENT);
    let stopped = chapter::header(&tools[3..], 200, false);
    assert_eq!(stopped.rows[0].spans[0].style, WARNING);
    let done = chapter::header(&tools[4..], 200, false);
    assert_eq!(done.rows[0].spans[0].style, GOOD);

    let cancelled_exit = Value::object([
        ("cancelled", Value::Bool(true)),
        ("exit_code", Value::number(101)),
    ]);
    let stopped = chapter::header(
        &[tool("bash", &args, Some(&cancelled_exit), &shown)],
        80,
        false,
    );
    assert_eq!(stopped.rows[0].spans[0].style, WARNING);
    assert!(plain(&stopped).contains("stopped"));
    assert_eq!(stopped.rows.len(), 2);
    assert_eq!(stopped.rows[1].plain(), "  ⎿ 1 stopped");
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
        assert!(!open.hidden);
        assert!(!plain(&open).contains("Ctrl+O"));
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
        assert!(text.starts_with("  ⎿ Bash  cargo test"), "{text}");
        assert!(text.contains(expected), "{text}");
        assert_eq!(rows.len(), 1, "{text}");
        assert!(!text.contains("p1") && !text.contains("missing import"));
        assert!(!text.contains('●') && !text.contains('▶'));
        assert_eq!(rows[0].spans[1].style, TOOL_ACCENT.bold());
        assert_eq!(rows[0].spans[2].style, crate::tui::theme::BODY);
        assert!(rows.iter().all(|row| text::cells(&row.plain()) <= 40));
    }
    let ok = Value::object([("exit_code", Value::number(0))]);
    let rows = chapter::exception(&tool("bash", &args, Some(&ok), &shown), 40);
    assert_eq!(rows[0].plain(), "  ⎿ Bash  cargo test");
    assert_eq!(rows.len(), 1);
    assert!(chapter::exception(&tool("read", &args, Some(&ok), &shown), 40).is_empty());
}

#[test]
fn compact_process_never_shows_ids_or_streams_and_activity_tracks_last_failure() {
    let args = object([("action", "poll"), ("process_id", "private-pid")]);
    let result = Value::object([
        ("process_id", Value::string("private-pid")),
        ("status", Value::string("running")),
        ("stdout", Value::string("raw process output")),
    ]);
    let shown = Presentation::default();
    let rows = chapter::exception(&tool("process", &args, Some(&result), &shown), 80);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].plain(), "  ⎿ Process  poll  background");
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
    assert!(activity.contains("Read  src/missing.rs"), "{activity}");
    assert!(activity.contains("failed"), "{activity}");
    assert!(!activity.contains("src/good.rs"), "{activity}");
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
        "● Verify  2 calls · 2 failed\n  ⎿ 0 completed"
    );
    let mixed = chapter::header(
        &[calls[0], tool("bash", &args, Some(&stopped), &shown)],
        80,
        false,
    );
    assert_eq!(plain(&mixed), "● Verify  2 calls · 1 failed\n  ⎿ 1 stopped");
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
        assert_eq!(rows[0].plain(), "  ⎿ Bash  cargo test");
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
