use super::*;
use crate::tui::theme::{BODY, CODE_TEXT};
use std::time::Duration;

fn object<const N: usize>(fields: [(&str, &str); N]) -> Value {
    Value::object(fields.map(|(key, value)| (key, Value::string(value))))
}
fn tool<'a>(
    name: &'a str,
    arguments: &'a Value,
    result: &'a Value,
    shown: &'a Presentation,
) -> Tool<'a> {
    Tool {
        name,
        arguments,
        result: Some(result),
        change: None,
        summary: "Script evaluated",
        presentation: shown,
    }
}
fn plain(rows: &[Line]) -> Vec<String> {
    rows.iter().map(Line::plain).collect()
}

#[test]
fn expanded_streams_keep_blank_lines_styles_complete_command_and_closing_facts() {
    let command = format!(
        "printf '{}' complete_command_tail",
        "long command argument ".repeat(10)
    );
    let args = object([
        ("command", &command),
        ("description", "Run a complete check"),
    ]);
    let result = Value::object([
        ("stdout", Value::string("\nfirst\n\nlast\n")),
        ("stderr", Value::string("note\n")),
        ("exit_code", Value::number(0)),
    ]);
    let shown = Presentation {
        expanded: true,
        elapsed: Some(Duration::from_millis(7900)),
        ..Presentation::default()
    };
    for columns in [80, 40] {
        let rows = card(tool("bash", &args, &result, &shown), columns, false).rows;
        let text = plain(&rows);
        assert!(text.iter().any(|row| row.contains("complete_command_tail")));
        let body = text.iter().position(|row| row == "    stdout").unwrap();
        assert_eq!(
            text[body..],
            [
                "    stdout",
                "    ",
                "    first",
                "    ",
                "    last",
                "    stderr",
                "    note",
                "    exit 0 · 7.9s"
            ]
        );
        for (label, style) in [
            ("stdout", TOOL_DIM),
            ("stderr", TOOL_DIM),
            ("first", CODE_TEXT),
            ("last", CODE_TEXT),
            ("note", MUTED),
        ] {
            assert!(
                rows.iter()
                    .flat_map(|row| &row.spans)
                    .any(|span| span.text == label && span.style == style)
            );
        }
        assert!(!text.iter().any(|row| row.contains("Done")));
        assert!(rows.iter().all(|row| text::cells(&row.plain()) <= columns));
    }
    let process = Value::object([
        ("exit_code", Value::number(42)),
        ("stderr", Value::string("failed\n")),
    ]);
    assert!(
        plain(&card(tool("process", &args, &process, &shown), 40, false).rows)
            .ends_with(&["    exit 42 · 7.9s".into()])
    );
    let browser = card(
        tool(
            "browser",
            &object([("action", "evaluate"), ("expression", "document.title")]),
            &object([("value", "ready")]),
            &shown,
        ),
        40,
        false,
    );
    assert_eq!(browser.rows.last().unwrap().plain(), "    7.9s");
    let with_notice = Value::object([
        ("exit_code", Value::number(0)),
        (
            "checkpoint",
            object([("status", "partial"), ("reason", "Untracked paths")]),
        ),
    ]);
    let rows = card(tool("bash", &args, &with_notice, &shown), 40, false).rows;
    assert!(
        plain(&rows)
            .iter()
            .any(|row| row.contains("Checkpoint partial"))
    );
    assert_eq!(rows.last().unwrap().plain(), "    exit 0 · 7.9s");
}

#[test]
fn new_file_preview_uses_retained_source_not_mutated_arguments_and_expands_all_lines() {
    let captured: String = (1..=16)
        .map(|n| format!("let captured_{n} = {n};\n"))
        .collect();
    let change = Change::between(None, &captured).unwrap();
    let args = object([("path", "src/new.rs"), ("content", "different arguments")]);
    let result = object([]);
    for columns in [80, 40] {
        for expanded in [false, true] {
            let shown = Presentation {
                expanded,
                ..Presentation::default()
            };
            let mut call = tool("write", &args, &result, &shown);
            call.change = Some(&change);
            let rendered = card(call, columns, false);
            let rows = plain(&rendered.rows);
            assert_eq!(rows[1], "  ⎿ New file · 16 lines");
            assert!(rows.iter().any(|row| row.contains("captured_6")));
            assert_eq!(rows.iter().any(|row| row.contains("captured_16")), expanded);
            assert!(
                !rows
                    .iter()
                    .any(|row| row.contains("different arguments") || row.contains("+  let"))
            );
            assert!(rendered.rows.iter().all(|row| row.background.is_none()));
            assert_eq!(rendered.hidden, !expanded);
            if !expanded {
                assert_eq!(rows.last().unwrap(), "    +10 lines");
            }
        }
    }
}

#[test]
fn file_paths_use_body_and_read_facts_do_not_repeat_the_verb() {
    let args = Value::object([
        ("path", Value::string("src/a.rs")),
        ("offset", Value::number(33)),
    ]);
    let result = Value::object([
        ("lines_returned", Value::number(9)),
        ("offset", Value::number(33)),
    ]);
    let shown = Presentation::default();
    let rows = card(tool("read", &args, &result, &shown), 80, false).rows;
    assert_eq!(rows[1].plain(), "  ⎿ 9 lines · from line 33");
    assert!(
        rows[0]
            .spans
            .iter()
            .any(|span| span.text == "src/a.rs" && span.style == BODY)
    );
    let skill = card(
        tool(
            "read",
            &object([("path", "skill:rust/SKILL.md")]),
            &result,
            &shown,
        ),
        80,
        false,
    );
    assert_eq!(
        skill.rows[1].plain(),
        "  ⎿ Loaded skill · 9 lines · from line 33"
    );
}

#[test]
fn expanded_command_errors_keep_the_complete_error_without_a_stdout_payload() {
    let args = object([
        ("command", "missing-command"),
        ("description", "Check availability"),
    ]);
    let result = object([("error", "Could not launch the command: executable missing")]);
    let shown = Presentation {
        expanded: true,
        ..Presentation::default()
    };
    let rows = card(tool("bash", &args, &result, &shown), 40, false).rows;
    let message = rows
        .iter()
        .filter(|row| row.origin.is_some_and(|origin| origin.line == 3))
        .map(|row| row.plain().trim().to_owned())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(message, "Could not launch the command: executable missing");
    assert!(
        rows.iter()
            .flat_map(|row| &row.spans)
            .any(|span| span.style == BAD && span.text.contains("Could not launch"))
    );
}
