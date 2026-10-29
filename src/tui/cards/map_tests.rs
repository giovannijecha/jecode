use super::*;

fn tool<'a>(args: &'a Value, result: &'a Value, presentation: &'a Presentation) -> Tool<'a> {
    Tool {
        name: "map",
        arguments: args,
        summary: "complete",
        result: Some(result),
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
fn overview_is_compact_and_expands_into_structured_entries_and_source_status() {
    let args = Value::object([("path", Value::string("src"))]);
    let result = Value::object([
        ("entries_returned", Value::number(2)),
        ("files", Value::number(12)),
        ("notes_count", Value::number(1)),
        ("complete", Value::Bool(false)),
        ("next_offset", Value::number(3)),
        ("errors_count", Value::number(1)),
        (
            "errors",
            Value::Array(vec![Value::object([
                ("path", Value::string("src/private.rs")),
                ("error", Value::string("Permission denied")),
            ])]),
        ),
        (
            "entries",
            Value::Array(vec![
                Value::object([
                    ("kind", Value::string("area")),
                    ("path", Value::string("src/tui")),
                    ("title", Value::string("Terminal UI")),
                    ("role", Value::string("Renders the conversation")),
                    ("origin", Value::string("filesystem")),
                    ("file_count", Value::number(27)),
                ]),
                Value::object([
                    ("kind", Value::string("note")),
                    ("title", Value::string("Keep renderer separate")),
                    ("summary", Value::string("Agent loop does not paint cells")),
                    ("origin", Value::string("model")),
                    ("freshness", Value::string("unverified")),
                    ("id", Value::string("n1")),
                    (
                        "sources",
                        Value::Array(vec![Value::object([
                            ("path", Value::string("src/tui/view.rs")),
                            ("line", Value::number(42)),
                            ("status", Value::string("unverified")),
                            ("error", Value::string("Fingerprint budget \u{1b}exhausted")),
                        ])]),
                    ),
                ]),
            ]),
        ),
    ]);
    for columns in [40, 80] {
        let compact = card(
            tool(&args, &result, &Presentation::default()),
            columns,
            true,
        );
        let text = plain(&compact);
        assert!(compact.hidden, "{text}");
        assert!(text.contains("2 entries"), "{text}");
        assert!(!text.contains("Terminal UI"), "{text}");
        let expanded = Presentation {
            expanded: true,
            ..Presentation::default()
        };
        let full = card(tool(&args, &result, &expanded), columns, true);
        let text = plain(&full);
        for needle in [
            "Terminal UI",
            "Keep renderer separate",
            "unverified",
            "src/tui/view.rs:42",
            "Fingerprint budget",
            "exhausted",
            "Permission",
            "denied",
            "More from 3",
        ] {
            assert!(text.contains(needle), "missing {needle}: {text}");
        }
        assert!(!text.contains("\"entries\""), "{text}");
        assert!(!text.contains('\u{1b}'), "{text}");
        assert!(text.contains("[ESC]"), "{text}");
        assert!(full.rows.iter().flat_map(|row| &row.spans).any(|span| {
            span.text.contains("Fingerprint budget") && span.style == crate::tui::theme::WARNING
        }));
        assert!(
            full.rows
                .iter()
                .all(|row| crate::tui::text::cells(&row.plain()) <= columns)
        );
    }
}

#[test]
fn empty_and_mutation_receipts_are_readable() {
    let args = Value::object([]);
    let expanded = Presentation {
        expanded: true,
        ..Presentation::default()
    };
    let empty = Value::object([
        ("entries", Value::Array(vec![])),
        ("complete", Value::Bool(true)),
    ]);
    assert!(plain(&card(tool(&args, &empty, &expanded), 40, false)).contains("No map entries"));
    let remember_args = Value::object([
        ("action", Value::string("remember")),
        ("title", Value::string("Auth boundary")),
    ]);
    let remembered = Value::object([
        ("action", Value::string("remember")),
        ("saved", Value::Bool(true)),
        (
            "note",
            Value::object([
                ("kind", Value::string("note")),
                ("id", Value::string("n1")),
                ("title", Value::string("Auth boundary")),
                ("freshness", Value::string("current")),
            ]),
        ),
    ]);
    let text = plain(&card(
        tool(&remember_args, &remembered, &expanded),
        40,
        false,
    ));
    assert!(text.contains("Auth boundary"), "{text}");
    assert!(text.contains("id n1"), "{text}");
    let forgotten = Value::object([
        ("action", Value::string("forget")),
        ("removed", Value::Bool(false)),
    ]);
    assert!(plain(&card(tool(&args, &forgotten, &expanded), 40, false)).contains("Note not found"));
    assert!(chapter::groupable(&tool(&args, &empty, &expanded)));
    assert!(!chapter::groupable(&tool(
        &remember_args,
        &remembered,
        &expanded
    )));
}
