use super::*;
use crate::tui::theme::{CODE_TEXT, DIFF_ADDED_SURFACE, KEYWORD};

mod files;
mod reads;

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
            "Browser(screenshot)",
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
        if name == "browser" {
            assert_eq!(rows[1].plain(), "  ⎿ tab-1");
        }
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
            .any(|span| span.text == "Bash" && span.style == TOOL_ACCENT.bold())
    );
    assert!(rows[0].background.is_none());
}

#[test]
fn expanded_titles_keep_long_unicode_commands_and_fit_narrow_views() {
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
        let title: Vec<_> = rows
            .iter()
            .filter(|row| row.origin.is_some_and(|origin| origin.line == 0))
            .collect();
        assert!(title[0].plain().starts_with("● Bash(cargo"));
        assert!(!title.iter().any(|row| row.plain().contains('…')));
        let retained = title
            .iter()
            .map(|row| row.plain().chars().skip(2).collect::<String>())
            .collect::<String>();
        assert_eq!(
            retained.split_whitespace().collect::<String>(),
            format!("Bash({command})")
                .split_whitespace()
                .collect::<String>()
        );
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
        let output: String = plain(&expanded)
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        assert!(output.contains("END"));
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
    assert!(plain(&rows).contains("⎿ Running · 2.4s"));
    assert!(rows[1..].iter().all(|row| row.background.is_none()));
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
            && !text.contains("more lines")
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
            .any(|span| span.text == "compiled one" && span.style == CODE_TEXT)
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
        if expanded {
            assert!(!text.contains("⎿ Done"));
            assert!(text.contains("exit 0"));
        } else {
            assert!(text.contains("⎿ last"));
        }
        assert_eq!(text.contains("+2 lines"), !expanded);
    }
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
        plain(&rows).contains("1 process · 1 running  +1 line")
            && !plain(&rows).contains("p1  running  cargo check")
    );
    let expanded = draw(
        "process",
        &arguments,
        Some(&list),
        None,
        80,
        &Presentation {
            expanded: true,
            ..Presentation::default()
        },
    );
    assert!(plain(&expanded).contains("p1  running  cargo check"));
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
    assert!(plain(&rows).contains("New file · 0 lines"));
}
