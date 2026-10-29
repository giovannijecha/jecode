// List and Search cards painted by the Rust terminal renderer at 40/80 cells:
// cargo test --locked --offline specimen::discovery::discovery_cards -- --ignored --nocapture
use crate::{
    events::tool_summary,
    json::Value,
    tui::{
        cards::{self, Presentation, Tool},
        line::Line,
        text,
        theme::{MUTED, STRONG},
    },
};

fn listing() -> (Value, Value) {
    (
        Value::object([
            ("path", Value::string("src")),
            ("description", Value::string("Inspect implementation files")),
        ]),
        Value::object([
            ("path", Value::string("src")),
            ("entries_returned", Value::number(3)),
            ("offset", Value::number(1)),
            ("next_offset", Value::number(4)),
            ("truncated", Value::Bool(true)),
            ("complete", Value::Bool(false)),
            (
                "entries",
                Value::Array(vec![
                    Value::object([
                        ("path", Value::string("src/agent.rs")),
                        ("type", Value::string("file")),
                        ("depth", Value::number(0)),
                        ("size", Value::number(24576)),
                    ]),
                    Value::object([
                        ("path", Value::string("src/tui")),
                        ("type", Value::string("directory")),
                        ("depth", Value::number(0)),
                    ]),
                    Value::object([
                        ("path", Value::string("src/tui/cards/discovery.rs")),
                        ("type", Value::string("file")),
                        ("depth", Value::number(1)),
                        ("size", Value::number(4096)),
                    ]),
                ]),
            ),
            ("errors", Value::Array(vec![])),
        ]),
    )
}

fn search() -> (Value, Value) {
    (
        Value::object([
            ("query", Value::string("tool_summary")),
            ("path", Value::string("src")),
            ("description", Value::string("Find summary usage")),
        ]),
        Value::object([
            ("path", Value::string("src")),
            ("query", Value::string("tool_summary")),
            ("matches_returned", Value::number(1)),
            ("files_searched", Value::number(12)),
            ("complete", Value::Bool(false)),
            ("errors_count", Value::number(2)),
            ("errors_truncated", Value::Bool(true)),
            (
                "matches",
                Value::Array(vec![Value::object([
                    ("path", Value::string("src/events.rs")),
                    ("line", Value::number(80)),
                    (
                        "text",
                        Value::string(
                            "pub fn tool_summary(name: &str, result: &Value) -> String {",
                        ),
                    ),
                    ("byte_offset", Value::number(2013)),
                    ("text_truncated", Value::Bool(false)),
                    (
                        "before",
                        Value::Array(vec![Value::object([
                            ("line", Value::number(79)),
                            ("text", Value::string("/// Summarize a tool result.")),
                            ("text_truncated", Value::Bool(false)),
                        ])]),
                    ),
                    (
                        "after",
                        Value::Array(vec![Value::object([
                            ("line", Value::number(81)),
                            (
                                "text",
                                Value::string("    let summary = outcome_summary(name, result);"),
                            ),
                            ("text_truncated", Value::Bool(false)),
                        ])]),
                    ),
                ])]),
            ),
            (
                "errors",
                Value::Array(vec![Value::object([
                    ("path", Value::string("src/private.rs")),
                    ("error", Value::string("Permission denied")),
                ])]),
            ),
        ]),
    )
}

fn card(name: &str, args: &Value, result: &Value, columns: usize, expanded: bool) -> Vec<Line> {
    let presentation = Presentation {
        expanded,
        ..Presentation::default()
    };
    let summary = tool_summary(name, result);
    let card = cards::card(
        Tool {
            name,
            arguments: args,
            summary: &summary,
            result: Some(result),
            change: None,
            presentation: &presentation,
        },
        columns,
        true,
    );
    assert!(
        card.rows
            .iter()
            .all(|row| text::cells(&row.plain()) <= columns)
    );
    card.rows
}

#[test]
#[ignore = "prints List and Search cards for review in a native terminal"]
fn discovery_cards() {
    let mut painted = String::new();
    for columns in [40, 80] {
        for (name, (args, result)) in [("list", listing()), ("search", search())] {
            for expanded in [false, true] {
                let heading = format!(
                    "{name} {} · {columns} columns",
                    if expanded { "expanded" } else { "compact" }
                );
                painted.push_str(&format!("{}\n", Line::new(&heading, STRONG).paint(columns)));
                let rows = card(name, &args, &result, columns, expanded);
                let plain = rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n");
                assert!(!plain.contains("\"matches\""));
                assert!(!plain.contains("\"entries\""));
                if expanded {
                    assert!(plain.contains(if name == "list" {
                        "src/agent.rs"
                    } else {
                        "src/events.rs:80"
                    }));
                } else {
                    assert!(plain.contains(if name == "list" {
                        "3 entries"
                    } else {
                        "2 errors"
                    }));
                }
                for row in rows {
                    painted.push_str(&format!("{}\n", row.paint(columns)));
                }
                painted.push_str(&format!("{}\n", Line::new("", MUTED).paint(columns)));
            }
        }
    }
    print!("{painted}");
}
