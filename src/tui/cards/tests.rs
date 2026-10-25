use super::*;
use crate::tui::theme::{BODY, DIFF_ADDED_SURFACE, KEYWORD};

fn draw(
    name: &str,
    arguments: &Value,
    result: Option<&Value>,
    change: Option<&Change>,
    columns: usize,
    presentation: &Presentation,
) -> Vec<Line> {
    render(
        Tool {
            name,
            arguments,
            summary: "complete",
            result,
            change,
            presentation,
        },
        columns,
    )
}
fn plain(rows: &[Line]) -> String {
    rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n")
}
fn args(key: &str, value: &str) -> Value {
    Value::object([(key, Value::string(value))])
}
fn ok() -> Value {
    Value::object([("exit_code", Value::number(0))])
}

#[test]
fn web_and_browser_cards_show_action_target_and_result() {
    let cases = [
        (
            "web",
            Value::object([
                ("action", Value::string("fetch")),
                ("url", Value::string("https://example.org/")),
            ]),
            Value::object([("status", Value::number(200)), ("bytes", Value::number(42))]),
            "Web(fetch https://example.org/)",
            "HTTP 200 · 42 B",
        ),
        (
            "browser",
            Value::object([
                ("action", Value::string("screenshot")),
                ("tab_id", Value::string("tab-1")),
            ]),
            Value::object([("screenshot_path", Value::string("tmp:browser/shot.png"))]),
            "Browser(screenshot tab-1)",
            "Screenshot · tmp:browser/shot.png",
        ),
    ];
    for (name, arguments, result, title, outcome) in cases {
        let rows = render(
            Tool {
                name,
                arguments: &arguments,
                summary: &crate::events::tool_summary(name, &result),
                result: Some(&result),
                change: None,
                presentation: &Presentation::default(),
            },
            80,
        );
        let text = plain(&rows);
        assert!(text.contains(title), "{text}");
        assert!(text.contains(outcome), "{text}");
    }
}

#[test]
fn title_and_result_are_left_aligned_without_a_tree_or_status_column() {
    let rows = draw(
        "bash",
        &args("command", "cargo test"),
        Some(&ok()),
        None,
        80,
        &Presentation::default(),
    );
    assert_eq!(
        rows.iter().map(Line::plain).collect::<Vec<_>>(),
        ["● Bash(cargo test)", "  ⎿ Done"]
    );
    assert_eq!(rows[0].spans[0].style, GOOD);
    assert!(
        rows[0]
            .spans
            .iter()
            .any(|span| span.text == "Bash(" && span.style == TOOL_ACCENT.bold())
    );
    assert!(rows[0].background.is_none());
}

#[test]
fn wrapped_titles_retain_long_unicode_commands_and_fit_narrow_views() {
    let command = "cargo test --locked --offline -- test_é_東京_🦀 --nocapture";
    for columns in [20, 40, 80, 120] {
        let rows = draw(
            "bash",
            &args("command", command),
            Some(&ok()),
            None,
            columns,
            &Presentation {
                expanded: true,
                ..Presentation::default()
            },
        );
        // Words wrap under the open paren; every character survives.
        let title: String = rows
            .iter()
            .filter(|row| row.origin.is_some_and(|origin| origin.line == 0))
            .flat_map(|row| row.plain().chars().skip(2).collect::<Vec<_>>())
            .filter(|character| !character.is_whitespace())
            .collect();
        let expected: String = format!("Bash({command})")
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        assert_eq!(title, expected);
        assert!(rows.iter().all(|row| text::cells(&row.plain()) <= columns));
    }
    for columns in 2..20 {
        let rows = draw(
            "read",
            &args("path", command),
            Some(&ok()),
            None,
            columns,
            &Presentation::default(),
        );
        assert!(rows.iter().all(|row| text::cells(&row.plain()) <= columns));
    }
}

#[test]
fn multiline_title_origins_include_line_breaks() {
    let rows = title::rows("Bash", "one\ntwo\nthree", TOOL_ACCENT, 80, true);
    let offsets: Vec<_> = rows
        .iter()
        .map(|row| row.origin.expect("title origin").character)
        .collect();
    assert_eq!(offsets, [0, 4, 8]);
}

#[test]
fn grouped_windows_reads_use_distinct_path_suffixes() {
    let arguments = [
        args("path", r"C:\shared\mod.rs"),
        args("path", r"D:\shared\mod.rs"),
    ];
    let result = Value::object([
        ("content", Value::string("1: item\n")),
        ("offset", Value::number(1)),
        ("lines_returned", Value::number(1)),
    ]);
    let presentation = Presentation::default();
    let reads: Vec<_> = arguments
        .iter()
        .map(|arguments| Tool {
            name: "read",
            arguments,
            summary: "complete",
            result: Some(&result),
            change: None,
            presentation: &presentation,
        })
        .collect();
    let rows = group(&reads, 100, false).rows;
    let text = plain(&rows);
    assert!(
        text.contains(r"C:\shared\mod.rs, D:\shared\mod.rs"),
        "{text}"
    );
}

#[test]
fn compact_long_lines_stay_bounded_and_expansion_retains_the_tail() {
    let text = format!("{}END", "界x".repeat(8_000));
    let arguments = args("command", &text);
    let result = Value::object([
        ("exit_code", Value::number(0)),
        ("stdout", Value::string(&text)),
    ]);
    for columns in [20, 40, 80] {
        let compact = draw(
            "bash",
            &arguments,
            Some(&result),
            None,
            columns,
            &Presentation::default(),
        );
        assert!(compact.len() <= 7);
        assert!(plain(&compact).contains('…'));
        assert!(!plain(&compact).contains("END"));
        let expanded = draw(
            "bash",
            &arguments,
            Some(&result),
            None,
            columns,
            &Presentation {
                expanded: true,
                ..Presentation::default()
            },
        );
        assert!(plain(&expanded).contains("END"));
        assert!(expanded.len() > compact.len());
    }
}

#[test]
fn running_states_stay_explicit_without_a_selection_mode() {
    let rows = draw(
        "bash",
        &args("command", "cargo build"),
        None,
        None,
        80,
        &Presentation {
            elapsed: Some(std::time::Duration::from_millis(2400)),
            ..Presentation::default()
        },
    );
    assert!(rows[0].background.is_none());
    assert_eq!(rows[0].spans[0].style, crate::tui::theme::TOOL_PULSE[2]);
    assert!(plain(&rows).contains("⎿ 2.4s"));
    assert!(rows[1..].iter().all(|row| row.background.is_none()));
}

#[test]
fn read_expansion_preserves_numbered_source_and_syntax() {
    let result = Value::object([
        (
            "content",
            Value::string("41: pub fn login() {\n42:     let limit = 10;\n"),
        ),
        ("offset", Value::number(41)),
        ("lines_returned", Value::number(2)),
    ]);
    let arguments = args("path", "src/auth.rs");
    let compact = draw(
        "read",
        &arguments,
        Some(&result),
        None,
        80,
        &Presentation::default(),
    );
    assert_eq!(compact.len(), 2);
    assert!(plain(&compact).contains("Read 2 lines · from line 41"));
    let rows = draw(
        "read",
        &arguments,
        Some(&result),
        None,
        40,
        &Presentation {
            expanded: true,
            ..Presentation::default()
        },
    );
    assert!(plain(&rows).contains("41   pub fn login()"));
    assert!(
        rows.iter()
            .flat_map(|row| &row.spans)
            .any(|span| span.style == KEYWORD && span.text == "pub")
    );
    assert!(!plain(&rows).contains("41:"));
    assert!(rows.iter().all(|row| text::cells(&row.plain()) <= 40));
}

#[test]
fn byte_reads_do_not_invent_line_numbers() {
    let arguments = Value::object([
        ("path", Value::string("output:s1:c1:stdout")),
        ("byte_offset", Value::number(16)),
    ]);
    let result = Value::object([
        ("offset", Value::Null),
        ("byte_offset", Value::number(16)),
        ("bytes_returned", Value::number(5)),
        ("content", Value::string("hello")),
    ]);
    let rows = draw(
        "read",
        &arguments,
        Some(&result),
        None,
        80,
        &Presentation {
            expanded: true,
            ..Presentation::default()
        },
    );
    assert!(plain(&rows).contains("Read 5 bytes · from byte 16"));
    assert_eq!(rows.last().unwrap().plain(), "    hello");
}

#[test]
fn failed_commands_prioritize_errors_and_expand_both_streams() {
    let result = Value::object([
        ("stdout", Value::string("compiled one\ncompiled two\n")),
        (
            "stderr",
            Value::string((1..=8).map(|n| format!("error {n}\n")).collect::<String>()),
        ),
        ("exit_code", Value::number(101)),
    ]);
    let arguments = args("command", "cargo build");
    let compact = draw(
        "bash",
        &arguments,
        Some(&result),
        None,
        80,
        &Presentation::default(),
    );
    let text = plain(&compact);
    assert!(
        text.contains("⎿ Exit 101 · error 1")
            && text.contains("    error 3")
            && !text.contains("error 4")
            && text.contains("7 more lines")
    );
    assert!(!text.contains("compiled one"));
    let rows = draw(
        "bash",
        &arguments,
        Some(&result),
        None,
        80,
        &Presentation {
            expanded: true,
            ..Presentation::default()
        },
    );
    let text = plain(&rows);
    assert!(
        text.contains("stderr")
            && text.contains("stdout")
            && text.contains("error 8")
            && text.contains("compiled one")
    );
    assert!(
        rows.iter()
            .flat_map(|row| &row.spans)
            .any(|span| span.text == "compiled one" && span.style == BODY)
    );
}

#[test]
fn capture_notices_survive_collapse_and_expansion() {
    let result = Value::object([
        ("stdout", Value::string("old\nfirst\n\nlast\n")),
        ("exit_code", Value::number(0)),
        ("stdout_truncated", Value::Bool(true)),
        ("stdout_bytes", Value::number(90000)),
        ("output_limit_bytes", Value::number(65536)),
        ("stdout_file", Value::string("output:s1:c1:stdout")),
    ]);
    for expanded in [false, true] {
        let rows = draw(
            "bash",
            &args("command", "cargo test"),
            Some(&result),
            None,
            80,
            &Presentation {
                expanded,
                ..Presentation::default()
            },
        );
        let text = plain(&rows);
        assert!(text.contains("last") && text.contains("full output saved"));
        assert_eq!(text.contains("    old"), expanded);
        assert!(text.contains(if expanded { "⎿ Done" } else { "⎿ last" }));
        assert_eq!(text.contains("2 more lines"), !expanded);
    }
}

#[test]
fn file_changes_survive_collapse_and_errors_do_not_show_success_diffs() {
    let change = Change::between(
        Some("fn login() {\n    old();\n}\n"),
        "fn login() {\n    new();\n    save();\n}\n",
    )
    .unwrap();
    let arguments = args("path", "src/auth.rs");
    let initial = draw(
        "edit",
        &arguments,
        Some(&ok()),
        Some(&change),
        80,
        &Presentation::default(),
    );
    assert!(plain(&initial).contains("● Update(src/auth.rs)\n  ⎿ +2 −1 lines"));
    assert!(initial.iter().any(|row| {
        row.spans
            .iter()
            .any(|span| span.background == Some(DIFF_ADDED_SURFACE))
    }));
    let collapsed = draw(
        "edit",
        &arguments,
        Some(&ok()),
        Some(&change),
        80,
        &Presentation {
            expanded: false,
            ..Presentation::default()
        },
    );
    assert_eq!(initial, collapsed);
    let failed = Value::object([(
        "error",
        Value::string("File is read-only; its content was not changed"),
    )]);
    let rows = draw(
        "edit",
        &arguments,
        Some(&failed),
        Some(&change),
        80,
        &Presentation::default(),
    );
    assert!(!plain(&rows).contains("new();") && !plain(&rows).contains("+2"));
    assert!(plain(&rows).contains("⎿ File is read-only"));
}

#[test]
fn old_sessions_keep_snippet_diffs_without_fake_file_coordinates_or_eof() {
    let arguments = Value::object([
        ("path", Value::string("legacy.rs")),
        ("old_text", Value::string("same\nold")),
        ("new_text", Value::string("same\nnew")),
    ]);
    let rows = draw(
        "edit",
        &arguments,
        Some(&ok()),
        None,
        80,
        &Presentation::default(),
    );
    let text = plain(&rows);
    assert!(
        text.contains("+1 −1 lines") && text.contains("Original file coordinates unavailable.")
    );
    assert!(
        !text.contains("No newline at end of file")
            && !text.contains("    1 −")
            && !text.contains("    1 +")
    );
    assert_eq!(text.matches("same").count(), 1);
}

#[test]
fn process_states_and_lists_name_their_actual_outcomes() {
    let arguments = Value::object([
        ("action", Value::string("poll")),
        ("process_id", Value::string("p1")),
    ]);
    for (status, expected) in [
        ("running", "⎿ p1 running"),
        ("stopping", "⎿ p1 stopping"),
        ("stopped", "⎿ p1 stopped"),
        ("unknown", "⎿ p1 status unknown"),
    ] {
        let result = Value::object([
            ("process_id", Value::string("p1")),
            ("status", Value::string(status)),
        ]);
        let rows = draw(
            "process",
            &arguments,
            Some(&result),
            None,
            80,
            &Presentation::default(),
        );
        assert_eq!(rows[0].plain(), "● Process(poll p1)");
        assert!(plain(&rows).contains(expected));
    }
    let list = Value::object([(
        "processes",
        Value::Array(vec![Value::object([
            ("process_id", Value::string("p1")),
            ("status", Value::string("running")),
            ("command", Value::string("cargo check")),
        ])]),
    )]);
    let rows = draw(
        "process",
        &arguments,
        Some(&list),
        None,
        80,
        &Presentation::default(),
    );
    assert!(
        plain(&rows).contains("1 process · 1 running")
            && plain(&rows).contains("p1  running  cargo check")
    );
}

#[test]
fn non_text_attachments_keep_their_view_and_expandable_metadata() {
    let arguments = args("path", "attachment:fixture");
    for (view, summary) in [("image", "Opened image"), ("pdf", "Opened PDF")] {
        let result = Value::object([
            ("view", Value::string(view)),
            (
                "note",
                Value::string("The attachment follows in the next user message."),
            ),
        ]);
        for expanded in [false, true] {
            let rows = draw(
                "read",
                &arguments,
                Some(&result),
                None,
                80,
                &Presentation {
                    expanded,
                    ..Presentation::default()
                },
            );
            let text = plain(&rows);
            assert!(text.contains(summary) && !text.contains("Read 0 lines"));
            assert_eq!(text.contains("next user message"), expanded);
        }
    }
    let binary = Value::object([("attachment", Value::object([]))]);
    let rows = draw(
        "read",
        &arguments,
        Some(&binary),
        None,
        80,
        &Presentation::default(),
    );
    assert!(plain(&rows).contains("Attachment available"));
}

#[test]
fn empty_creations_and_unavailable_previous_content_are_honest() {
    let arguments = Value::object([
        ("path", Value::string("notes.txt")),
        ("content", Value::string("hello\n")),
    ]);
    let rows = draw(
        "write",
        &arguments,
        Some(&ok()),
        None,
        80,
        &Presentation::default(),
    );
    assert!(plain(&rows).contains("⎿ Wrote 1 line"));
    assert!(rows.iter().all(|row| row.background.is_none()));
    let change = Change::between(None, "").unwrap();
    let rows = draw(
        "write",
        &arguments,
        Some(&ok()),
        Some(&change),
        80,
        &Presentation::default(),
    );
    assert_eq!(rows.len(), 2);
    assert!(plain(&rows).contains("Created an empty file"));
}
