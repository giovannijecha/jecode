use super::*;
use crate::tui::theme::{
    BADGE_IDLE, BADGE_SHELL, BADGE_TEXT, CODE_TEXT, DIFF_ADDED_SURFACE, KEYWORD,
};

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
            " WEB  fetch https://example.org/",
            "HTTP 200 · 42 B",
        ),
        (
            "browser",
            Value::object([
                ("action", Value::string("screenshot")),
                ("tab_id", Value::string("tab-1")),
            ]),
            Value::object([("screenshot_path", Value::string("tmp:browser/shot.png"))]),
            " BROWSER  screenshot",
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
        assert!(text.starts_with(title), "{text}");
        assert!(text.contains(outcome), "{text}");
        assert!(rows.last().unwrap().plain().starts_with("  └ "), "{text}");
        if name == "browser" {
            assert_eq!(rows[1].plain(), "  │ tab-1");
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
        [" BASH  cargo test", "  └ Done"]
    );
    // Done is the badge on the shell family surface; no status dot.
    assert_eq!(rows[0].spans[0].text, " BASH ");
    assert_eq!(rows[0].spans[0].style, BADGE_TEXT.bold());
    assert_eq!(rows[0].spans[0].background, Some(BADGE_SHELL));
    assert!(!plain(&rows).contains('●'));
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
        assert!(title[0].plain().starts_with(" BASH  cargo"));
        assert!(!title.iter().any(|row| row.plain().contains('…')));
        // Wrapped rows hang two cells in, under the badge.
        assert!(title[1..].iter().all(|row| row.plain().starts_with("  ")));
        let retained = title.iter().map(|row| row.plain()).collect::<String>();
        assert_eq!(
            retained.split_whitespace().collect::<String>(),
            format!("BASH {command}")
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
    let rows = title::rows("Bash", "one\ntwo\nthree", Status::Done, 80, true);
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
    // Running is the quiet badge with the shimmer across its name.
    assert_eq!(rows[0].spans[0].background, Some(BADGE_IDLE));
    assert_eq!(rows[0].spans[1].text, "BASH");
    assert_eq!(rows[0].spans[1].background, Some(BADGE_IDLE));
    assert!(plain(&rows).contains("└ Running · 2.4s"));
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
        text.contains("  │ Exit 101 · error 1")
            && text.contains("  └ error 3")
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
        assert_eq!(text.contains("  │ old"), expanded);
        if expanded {
            assert!(!text.contains("Done"));
            assert!(text.contains("└ exit 0"));
        } else {
            assert!(text.contains("  │ last"));
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
        ("running", "└ p1 running"),
        ("stopping", "└ p1 stopping"),
        ("stopped", "└ p1 stopped"),
        ("unknown", "└ p1 status unknown"),
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
        // Only a process still in the background ends its target in `&`.
        let background = matches!(status, "running" | "stopping");
        assert_eq!(
            rows[0].plain(),
            if background {
                " PROCESS  poll p1 &"
            } else {
                " PROCESS  poll p1"
            }
        );
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
    assert!(plain(&rows).starts_with(" WRITE  notes.txt  Wrote 1 line"));
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
    // The fact joins the title and no preview rows are invented.
    assert_eq!(rows.len(), 1);
    assert_eq!(plain(&rows), " WRITE  notes.txt  New file · 0 lines");
}

#[test]
fn a_null_error_field_is_not_a_failure() {
    // `agent inspect` returns the collaborator record, whose `error` is null.
    let record = Value::object([
        ("id", Value::string("a2")),
        ("state", Value::string("returned")),
        ("error", Value::Null),
    ]);
    let arguments = Value::object([
        ("action", Value::string("inspect")),
        ("id", Value::string("a2")),
    ]);
    let text = plain(&draw(
        "agent",
        &arguments,
        Some(&record),
        None,
        80,
        &Presentation::default(),
    ));
    assert!(!text.contains("Exit"), "{text}");
    let failed = Value::object([("error", Value::string("Unknown collaborator"))]);
    let text = plain(&draw(
        "agent",
        &arguments,
        Some(&failed),
        None,
        80,
        &Presentation::default(),
    ));
    assert!(text.contains("Unknown collaborator"), "{text}");
}
