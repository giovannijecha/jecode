// Native Map card specimen: cargo test --locked --offline specimen::map::map_cards -- --ignored --nocapture
use crate::{
    events::tool_summary,
    json::Value,
    tui::{
        cards::{self, Presentation, Tool},
        line::Line,
        text,
        theme::STRONG,
    },
};

#[test]
#[ignore = "prints Map cards for review in a native terminal"]
fn map_cards() {
    let args = Value::object([("path", Value::string("src"))]);
    let note = Value::object([
        ("kind", Value::string("note")),
        ("title", Value::string("Agent loop is renderer independent")),
        (
            "summary",
            Value::string("The terminal paints its own view of events."),
        ),
        ("origin", Value::string("model")),
        ("freshness", Value::string("changed")),
        ("id", Value::string("n1")),
        (
            "sources",
            Value::Array(vec![Value::object([
                ("path", Value::string("src/agent.rs")),
                ("line", Value::number(12)),
                ("status", Value::string("changed")),
            ])]),
        ),
    ]);
    let overview = Value::object([
        ("entries_returned", Value::number(2)),
        ("files", Value::number(12)),
        ("notes_count", Value::number(1)),
        (
            "entries",
            Value::Array(vec![
                Value::object([
                    ("kind", Value::string("area")),
                    ("path", Value::string("src/tui")),
                    ("title", Value::string("Terminal UI")),
                    ("role", Value::string("Conversation rendering")),
                    ("origin", Value::string("filesystem")),
                    ("file_count", Value::number(43)),
                ]),
                note.clone(),
            ]),
        ),
        ("complete", Value::Bool(true)),
    ]);
    let empty = Value::object([
        ("entries_returned", Value::number(0)),
        ("files", Value::number(0)),
        ("entries", Value::Array(vec![])),
        ("complete", Value::Bool(true)),
    ]);
    let remembered = Value::object([
        ("action", Value::string("remember")),
        ("saved", Value::Bool(true)),
        ("note", note),
    ]);
    let remember_args = Value::object([
        ("action", Value::string("remember")),
        ("title", Value::string("Agent loop is renderer independent")),
    ]);
    let mut painted = String::new();
    for columns in [40, 80] {
        for (label, args, result) in [
            ("overview", &args, &overview),
            ("empty", &args, &empty),
            ("remember", &remember_args, &remembered),
        ] {
            for expanded in [false, true] {
                painted.push_str(&format!(
                    "{}\n",
                    Line::new(
                        &format!(
                            "Map {label} · {columns} columns · {}",
                            if expanded { "expanded" } else { "compact" }
                        ),
                        STRONG
                    )
                    .paint(columns)
                ));
                let presentation = Presentation {
                    expanded,
                    ..Presentation::default()
                };
                let summary = tool_summary("map", result);
                let card = cards::card(
                    Tool {
                        name: "map",
                        arguments: args,
                        summary: &summary,
                        result: Some(result),
                        change: None,
                        presentation: &presentation,
                    },
                    columns,
                    true,
                );
                if label == "empty" && !expanded {
                    assert_eq!(card.rows[0].plain(), " MAP  src");
                }
                if label == "empty" && expanded {
                    assert_eq!(card.rows[1].plain(), "  └ No map entries");
                }
                assert!(
                    card.rows
                        .iter()
                        .all(|row| text::cells(&row.plain()) <= columns)
                );
                for row in card.rows {
                    painted.push_str(&format!("{}\n", row.paint(columns)));
                }
                painted.push('\n');
            }
        }
    }
    print!("{painted}");
}
