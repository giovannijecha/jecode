use super::*;
use crate::tui::theme::{BAD, WARNING};
use std::time::Duration;

fn draw(
    name: &str,
    arguments: &Value,
    result: &Value,
    columns: usize,
    expanded: bool,
) -> Vec<Line> {
    card(
        Tool {
            name,
            arguments,
            result: Some(result),
            summary: "Complete",
            change: None,
            presentation: &Presentation {
                expanded,
                elapsed: Some(Duration::from_secs(6)),
                ..Presentation::default()
            },
        },
        columns,
        true,
    )
    .rows
}

fn colored(rows: &[Line], style: Style) -> String {
    rows.iter()
        .flat_map(|row| &row.spans)
        .filter(|span| span.background.is_none() && span.style == style)
        .map(|span| span.text.as_str())
        .collect()
}

fn quiet_details(rows: &[Line]) {
    for span in rows.iter().flat_map(|row| &row.spans) {
        if span.background.is_none() && span.text != "Ctrl+O" {
            assert_eq!(span.style, DIM, "unexpected detail ink on {:?}", span.text);
        }
    }
}

#[test]
fn routine_cards_keep_their_complete_details_quiet_at_every_width() {
    let command = Value::object([
        ("command", Value::string("printf 'green'")),
        ("description", Value::string("Inspect the result")),
    ]);
    let output = Value::object([
        ("stdout", Value::string("green\n\ncomplete\n")),
        ("stderr", Value::string("additional information\n")),
        ("exit_code", Value::number(0)),
    ]);
    let read = Value::object([("path", Value::string("src/green.rs"))]);
    let source = Value::object([
        ("content", Value::string("1: pub const GREEN: u8 = 1;\n")),
        ("offset", Value::number(1)),
    ]);
    let browser = Value::object([
        ("action", Value::string("evaluate")),
        ("expression", Value::string("const title = 'green'; title")),
    ]);
    let web = Value::object([("url", Value::string("https://example.org/"))]);
    let page = Value::object([
        ("title", Value::string("Green page")),
        ("status", Value::number(200)),
        ("preview", Value::string("A complete result.")),
    ]);
    for (name, arguments, result) in [
        ("bash", &command, &output),
        ("process", &command, &output),
        ("read", &read, &source),
        ("browser", &browser, &output),
        ("web", &web, &page),
    ] {
        for columns in [44, 80, 120] {
            for expanded in [false, true] {
                let rows = draw(name, arguments, result, columns, expanded);
                quiet_details(&rows);
                assert!(rows.iter().all(|row| text::cells(&row.plain()) <= columns));
            }
        }
    }
}

#[test]
fn failed_command_colors_only_its_outcome_and_keeps_expanded_output() {
    let arguments = Value::object([("command", Value::string("cargo test"))]);
    let result = Value::object([
        ("exit_code", Value::number(101)),
        (
            "stderr",
            Value::string("error: a long diagnostic\n  --> src/main.rs:42\nsecond diagnostic\n"),
        ),
    ]);
    for columns in [44, 80, 120] {
        for expanded in [false, true] {
            let rows = draw("bash", &arguments, &result, columns, expanded);
            assert_eq!(
                colored(&rows, BAD),
                if expanded { "exit 101" } else { "Exit 101" }
            );
            assert_eq!(colored(&rows, WARNING), "");
            if expanded {
                let plain = rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n");
                assert!(plain.contains("second diagnostic"));
                assert!(plain.contains("exit 101 · 6.0s"));
            }
        }
    }
}

#[test]
fn stopped_commands_keep_one_marker_including_in_the_expanded_closure() {
    let arguments = Value::object([("command", Value::string("cargo test"))]);
    for (flag, marker) in [
        ("cancelled", "Cancelled after 6.0s"),
        ("timed_out", "Timed out after 6.0s"),
    ] {
        let result = Value::object([
            (flag, Value::Bool(true)),
            ("stdout", Value::string("partial output\n")),
        ]);
        for expanded in [false, true] {
            let rows = draw("bash", &arguments, &result, 80, expanded);
            assert_eq!(colored(&rows, WARNING), marker);
            assert_eq!(colored(&rows, BAD), "");
            if expanded {
                assert_eq!(
                    rows.last().unwrap().plain(),
                    format!("  └ {marker} · partial output")
                );
            }
        }
    }
}

#[test]
fn refused_non_shell_call_closes_with_its_outcome_instead_of_only_time() {
    let arguments = Value::object([]);
    let result = Value::object([(
        "error",
        Value::string("Refused before dispatch · invalid step"),
    )]);
    for expanded in [false, true] {
        let rows = draw("steps", &arguments, &result, 80, expanded);
        assert_eq!(colored(&rows, BAD), "Refused before dispatch");
        if expanded {
            assert!(
                rows.last()
                    .unwrap()
                    .plain()
                    .contains("Refused before dispatch · 6.0s")
            );
        }
    }
}

#[test]
fn batched_read_keeps_paths_and_source_quiet_and_marks_only_the_unread_count() {
    let arguments = Value::object([(
        "paths",
        Value::Array(vec![Value::string("good.rs"), Value::string("missing.rs")]),
    )]);
    let result = Value::object([(
        "files",
        Value::Array(vec![
            Value::object([
                ("path", Value::string("good.rs")),
                ("content", Value::string("1: pub fn ready() {}\n")),
            ]),
            Value::object([
                ("path", Value::string("missing.rs")),
                ("error", Value::string("Not found")),
            ]),
        ]),
    )]);
    for expanded in [false, true] {
        let rows = draw("read", &arguments, &result, 120, expanded);
        assert_eq!(colored(&rows, WARNING), "1 not read");
        assert_eq!(colored(&rows, BAD), "");
        for span in rows
            .iter()
            .flat_map(|row| &row.spans)
            .filter(|span| span.text.contains(".rs"))
        {
            assert_eq!(span.style, DIM);
        }
    }
}

#[test]
fn new_file_preview_keeps_syntax_while_its_title_and_facts_are_quiet() {
    let source = "pub const GREEN: &str = \"ready\";\n";
    let arguments = Value::object([
        ("path", Value::string("src/green.rs")),
        ("content", Value::string(source)),
    ]);
    let result = Value::object([("bytes_written", Value::number(source.len()))]);
    let change = Change::between(None, source).unwrap();
    for expanded in [false, true] {
        let rows = card(
            Tool {
                name: "write",
                arguments: &arguments,
                summary: "created",
                result: Some(&result),
                change: Some(&change),
                presentation: &Presentation {
                    expanded,
                    ..Presentation::default()
                },
            },
            80,
            true,
        )
        .rows;
        assert!(rows[0].plain().contains("SAVE"));
        assert!(
            rows.iter()
                .flat_map(|row| &row.spans)
                .any(|span| span.style == crate::tui::theme::KEYWORD && span.text == "pub")
        );
        for span in rows
            .iter()
            .flat_map(|row| &row.spans)
            .filter(|span| span.text.contains("green.rs") || span.text.contains("New file"))
        {
            assert_eq!(span.style, DIM);
        }
    }
}

#[test]
fn unstructured_diagnostics_and_refusal_paths_never_receive_state_ink() {
    let arguments = Value::object([("path", Value::string("src/private.rs"))]);
    let reason = "Not available at the Read permission level: editing src/private.rs";
    for (name, result, expected, tone) in [
        (
            "steps",
            Value::object([("error", Value::string("src/private.rs: invalid step"))]),
            "Failed",
            BAD,
        ),
        (
            "edit",
            Value::object([
                ("outcome", Value::string("unavailable")),
                ("reason", Value::string(reason)),
            ]),
            "Not run",
            WARNING,
        ),
        (
            "mcp",
            Value::object([("error", Value::string("src/private.rs: remote error"))]),
            "Failed",
            BAD,
        ),
        (
            "lsp",
            Value::object([
                ("status", Value::string("failed")),
                ("error", Value::string("src/private.rs: server error")),
            ]),
            "Failed",
            BAD,
        ),
    ] {
        for expanded in [false, true] {
            let rows = draw(name, &arguments, &result, 120, expanded);
            assert_eq!(
                colored(&rows, tone),
                expected,
                "{name}, expanded={expanded}"
            );
            if name == "lsp" && expanded {
                let plain = rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n");
                assert_eq!(plain.matches("src/private.rs: server error").count(), 1);
                assert_eq!(rows.last().unwrap().plain(), "  └ Failed");
            }
            assert!(
                rows.iter()
                    .flat_map(|row| &row.spans)
                    .filter(|span| span.text.contains("private.rs"))
                    .all(|span| span.style == DIM)
            );
        }
    }
}

#[test]
fn an_expanded_persistent_exit_reports_its_outcome_once_on_the_closing_row() {
    let arguments = Value::object([("executable", Value::string("helper.exe"))]);
    for (code, marker) in [(Value::number(7), "Exited 7"), (Value::Null, "Exited")] {
        let result = Value::object([
            ("launch", Value::string("persistent")),
            ("application", Value::string("exited")),
            ("pid", Value::number(4123)),
            ("exit_code", code),
        ]);
        let rows = draw("exec", &arguments, &result, 80, true);
        let plain = rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n");
        assert_eq!(plain.matches(marker).count(), 1);
        assert_eq!(colored(&rows, BAD), marker);
        assert_eq!(
            rows.last().unwrap().plain(),
            format!("  └ {marker} · pid 4123 · 6.0s")
        );
    }
}

#[test]
fn an_unknown_agent_marks_the_outcome_and_keeps_the_recovery_instruction_quiet() {
    let arguments = Value::object([("label", Value::string("AGENT 2"))]);
    let result = Value::object([("kind", Value::string("unknown"))]);
    for expanded in [false, true] {
        let rows = draw("collaboration", &arguments, &result, 120, expanded);
        assert_eq!(colored(&rows, WARNING), "Outcome unknown");
        assert!(
            rows.iter()
                .flat_map(|row| &row.spans)
                .filter(|span| span.text.contains("/agents"))
                .all(|span| span.style == DIM)
        );
    }
}
