use super::*;

fn tool<'a>(
    name: &'a str,
    arguments: &'a Value,
    result: &'a Value,
    presentation: &'a Presentation,
) -> Tool<'a> {
    Tool {
        name,
        arguments,
        summary: "complete",
        result: Some(result),
        change: None,
        presentation,
    }
}

fn text(card: &Card) -> String {
    card.rows
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn list_compact_and_expanded_show_entries_without_json() {
    let args = Value::object([
        ("path", Value::string("src")),
        ("description", Value::string("Inspect source tree")),
    ]);
    let result = Value::object([
        ("entries_returned", Value::number(2)),
        ("complete", Value::Bool(false)),
        ("next_offset", Value::number(3)),
        (
            "entries",
            Value::Array(vec![
                Value::object([
                    ("path", Value::string("src/agent.rs")),
                    ("type", Value::string("file")),
                    ("depth", Value::number(0)),
                    ("size", Value::number(812)),
                ]),
                Value::object([
                    ("path", Value::string("src/tui")),
                    ("type", Value::string("directory")),
                    ("depth", Value::number(1)),
                ]),
            ]),
        ),
    ]);
    for columns in [40, 80] {
        let collapsed = card(
            tool("list", &args, &result, &Presentation::default()),
            columns,
            true,
        );
        let compact = text(&collapsed);
        assert!(collapsed.hidden, "{compact}");
        assert!(compact.contains("Inspect source tree"), "{compact}");
        assert!(compact.contains("2 entries"), "{compact}");
        assert!(compact.contains("more from 3"), "{compact}");
        assert!(!compact.contains("\"entries\""), "{compact}");
        assert!(!compact.contains("src/agent.rs"), "{compact}");
        let expanded = Presentation {
            expanded: true,
            ..Presentation::default()
        };
        let open = card(tool("list", &args, &result, &expanded), columns, true);
        let full = text(&open);
        assert!(full.contains("src/agent.rs"), "{full}");
        assert!(full.contains("src/tui"), "{full}");
        assert!(full.contains("More from 3"), "{full}");
        assert!(!full.contains("\"entries\""), "{full}");
        assert!(
            open.rows
                .iter()
                .all(|line| crate::tui::text::cells(&line.plain()) <= columns)
        );
    }
}

#[test]
fn search_no_matches_and_partial_errors_remain_legible() {
    let args = Value::object([("query", Value::string("needle"))]);
    let empty = Value::object([
        ("matches_returned", Value::number(0)),
        ("files_searched", Value::number(6)),
        ("complete", Value::Bool(true)),
    ]);
    let expanded = Presentation {
        expanded: true,
        ..Presentation::default()
    };
    let no_matches = text(&card(tool("search", &args, &empty, &expanded), 40, false));
    assert!(no_matches.contains("No matches"), "{no_matches}");
    let partial = Value::object([
        ("matches_returned", Value::number(1)),
        ("files_searched", Value::number(2)),
        ("complete", Value::Bool(false)),
        ("errors_count", Value::number(4)),
        ("skipped", Value::object([("binary", Value::number(1))])),
        (
            "errors",
            Value::Array(vec![Value::object([
                ("path", Value::string("src/private.rs")),
                ("error", Value::string("Permission denied")),
            ])]),
        ),
        (
            "matches",
            Value::Array(vec![Value::object([
                ("path", Value::string("src/agent.rs")),
                ("line", Value::number(27)),
                ("text", Value::string("let needle = lookup();")),
                ("text_truncated", Value::Bool(false)),
                (
                    "before",
                    Value::Array(vec![Value::object([
                        ("line", Value::number(26)),
                        ("text", Value::string("// caller")),
                    ])]),
                ),
            ])]),
        ),
    ]);
    for columns in [40, 80] {
        let compact = text(&card(
            tool("search", &args, &partial, &Presentation::default()),
            columns,
            false,
        ));
        assert!(compact.contains("1 match"), "{compact}");
        assert!(compact.contains("4 errors"), "{compact}");
        let card = card(tool("search", &args, &partial, &expanded), columns, false);
        let full = text(&card);
        assert!(full.contains("src/agent.rs:27"), "{full}");
        assert!(full.contains("<26: // caller"), "{full}");
        assert!(full.contains("Skipped: binary 1"), "{full}");
        assert!(full.contains("Error src/private.rs: Permission"), "{full}");
        assert!(full.contains("denied"), "{full}");
        assert!(full.contains("3 more errors not shown"), "{full}");
        assert!(full.contains("Partial result"), "{full}");
        assert!(
            card.rows
                .iter()
                .all(|line| crate::tui::text::cells(&line.plain()) <= columns)
        );
    }
}
