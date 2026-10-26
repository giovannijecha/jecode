//! Native before/after evidence for file cards and expanded streams.
use crate::{
    file_change::Change,
    json::Value,
    tui::{
        cards::{self, Presentation, Tool},
        line::Line,
        theme::STRONG,
    },
};
use std::time::Duration;

struct Case {
    name: &'static str,
    arguments: Value,
    result: Value,
    change: Option<Change>,
}
fn object<const N: usize>(fields: [(&str, &str); N]) -> Value {
    Value::object(fields.map(|(key, value)| (key, Value::string(value))))
}
fn cases() -> Vec<Case> {
    let before: String = (1..=32)
        .map(|line| format!("let value_{line} = {line};\n"))
        .collect();
    let after = before
        .replace(
            "let value_3 = 3;",
            &format!(
                "let value_3 = format!(\"{}\");",
                "long source row ".repeat(9)
            ),
        )
        .replace(
            "let value_27 = 27;",
            "let value_27 = 270;\nverify(value_27);",
        );
    let fresh = "use std::time::Duration;\n\npub fn delay() -> Duration {\n    let milliseconds = 120;\n    Duration::from_millis(milliseconds)\n}\n\n// Captured source stays available.\n";
    let mut cases = vec![
        Case {
            name: "edit",
            arguments: object([("path", "src/parser.rs")]),
            result: object([]),
            change: Change::between(Some(&before), &after),
        },
        Case {
            name: "write",
            arguments: object([("path", "src/delay.rs"), ("content", fresh)]),
            result: object([]),
            change: Change::between(None, fresh),
        },
        Case {
            name: "write",
            arguments: object([
                ("path", "src/existing.rs"),
                ("content", "pub fn ready() {\n    println!(\"ready\");\n}\n"),
            ]),
            result: object([]),
            change: Change::between(
                Some("pub fn ready() {\n    old();\n}\n"),
                "pub fn ready() {\n    println!(\"ready\");\n}\n",
            ),
        },
        Case {
            name: "edit",
            arguments: object([
                ("path", "src/parser.rs"),
                ("old_text", "missing()"),
                ("new_text", "ready()"),
            ]),
            result: object([("error", "Expected one occurrence; found 0")]),
            change: None,
        },
        Case {
            name: "edit",
            arguments: object([
                ("path", "src/parser.rs"),
                ("old_text", "ready()"),
                ("new_text", "ready()"),
            ]),
            result: object([]),
            change: Change::between(Some("ready()\n"), "ready()\n"),
        },
        Case {
            name: "read",
            arguments: Value::object([
                ("path", Value::string("src/parser.rs")),
                ("offset", Value::number(33)),
            ]),
            result: Value::object([
                (
                    "content",
                    Value::string(
                        (33..42)
                            .map(|line| format!("{line}: let value_{line} = {line};\n"))
                            .collect::<String>(),
                    ),
                ),
                ("offset", Value::number(33)),
                ("lines_returned", Value::number(9)),
                ("next_offset", Value::number(42)),
            ]),
            change: None,
        },
        Case {
            name: "read",
            arguments: object([("path", "reference.png")]),
            result: Value::object([
                ("view", Value::string("image")),
                (
                    "attachment",
                    Value::object([
                        ("id", Value::string("image-reference")),
                        ("mime", Value::string("image/png")),
                        ("width", Value::number(800)),
                        ("height", Value::number(600)),
                        ("size", Value::number(86016)),
                    ]),
                ),
            ]),
            change: None,
        },
        Case {
            name: "bash",
            arguments: object([
                (
                    "command",
                    "cargo test --locked --offline --all-targets -- tui::cards::tinted_tests --nocapture && printf 'complete source and captured output remain available'",
                ),
                ("description", "Check tinted file cards"),
            ]),
            result: Value::object([
                ("exit_code", Value::number(0)),
                (
                    "stdout",
                    Value::string("\nchecking wrapping\n\nall checks passed\n"),
                ),
                ("stderr", Value::string("note: synthetic fixture\n")),
            ]),
            change: None,
        },
    ];
    cases.push(Case {
        name: "browser",
        arguments: object([
            ("action", "evaluate"),
            ("expression", "({title: document.title, width: innerWidth})"),
        ]),
        result: Value::object([
            ("value", object([("title", "Local preview")])),
            ("status", Value::string("evaluated")),
        ]),
        change: None,
    });
    cases
}

#[test]
#[ignore = "prints real file cards and expanded streams at 80 and 40 cells"]
fn tinted() {
    for columns in [80, 40] {
        for expanded in [false, true] {
            println!(
                "TINTED {columns} {}",
                if expanded { "expanded" } else { "compact" }
            );
            println!(
                "{}",
                Line::new("Jecode file cards and streams", STRONG.bold()).paint(columns)
            );
            for case in cases() {
                let shown = Presentation {
                    expanded,
                    elapsed: Some(Duration::from_millis(7900)),
                    ..Presentation::default()
                };
                let card = cards::card(
                    Tool {
                        name: case.name,
                        arguments: &case.arguments,
                        result: Some(&case.result),
                        summary: "Script evaluated",
                        change: case.change.as_ref(),
                        presentation: &shown,
                    },
                    columns,
                    false,
                );
                for line in card.rows {
                    assert!(crate::tui::text::cells(&line.plain()) <= columns);
                    println!("{}", line.paint(columns));
                }
                println!();
            }
        }
    }
}
