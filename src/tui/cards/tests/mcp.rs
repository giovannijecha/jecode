use super::*;
use crate::tui::theme::{BADGE_WARN, WARNING};

fn call() -> Value {
    Value::object([
        ("connection", Value::string("mcp-fixture-id")),
        ("connection_name", Value::string("GitHub")),
        ("action", Value::string("call")),
        ("tool", Value::string("create_issue")),
        (
            "arguments",
            Value::object([("title", Value::string("Fix sleep"))]),
        ),
        ("work", Value::string("Track issues")),
    ])
}

#[test]
fn mcp_cards_follow_the_shared_rhythm_and_preserve_remote_uncertainty() {
    let args = call();
    let done = Value::object([(
        "result",
        Value::object([(
            "content",
            Value::Array(vec![Value::object([
                ("type", Value::string("text")),
                ("text", Value::string("Created #7")),
            ])]),
        )]),
    )]);
    let unknown = Value::object([
        ("outcome", Value::string("unknown")),
        ("error", Value::string("Disconnected after dispatch")),
    ]);
    for width in [44, 80, 120] {
        let rows = draw(
            "mcp",
            &args,
            Some(&done),
            None,
            width,
            &Presentation::default(),
        );
        let value = plain(&rows);
        assert!(value.starts_with(" MCP  GitHub › create_issue"), "{value}");
        assert!(value.contains("│ title \"Fix sleep\""), "{value}");
        assert!(value.contains("└ Created #7"), "{value}");
        assert_eq!(rows[0].spans[0].background, Some(BADGE_IDLE));
        let rows = draw(
            "mcp",
            &args,
            Some(&unknown),
            None,
            width,
            &Presentation::default(),
        );
        assert_eq!(rows[0].spans[0].background, Some(BADGE_WARN));
        assert!(plain(&rows).contains("Outcome unknown"));
        assert!(
            rows.last()
                .unwrap()
                .spans
                .iter()
                .any(|span| span.style == WARNING)
        );
        assert!(rows.iter().all(|row| text::cells(&row.plain()) <= width));
        let expanded = Presentation {
            expanded: true,
            ..Presentation::default()
        };
        let rows = draw("mcp", &args, Some(&unknown), None, width, &expanded);
        let value = plain(&rows);
        let logical = value
            .lines()
            .map(|line| line.trim().trim_start_matches(['│', '└']).trim())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(logical.contains("Disconnected after dispatch"), "{value}");
        assert!(value.contains("Outcome unknown"), "{value}");
        assert!(rows.iter().all(|row| text::cells(&row.plain()) <= width));
    }
}

#[test]
fn work_keeps_each_mcp_connection_and_result_visible() {
    let mut args = call();
    if let Value::Object(fields) = &mut args {
        // Saved calls retain the stable ID without presentation-only fields.
        fields.remove("connection_name");
    }
    let mut result = Value::object([(
        "result",
        Value::object([(
            "content",
            Value::Array(vec![Value::object([
                ("type", Value::string("text")),
                ("text", Value::string("Created #7")),
            ])]),
        )]),
    )]);
    if let Value::Object(fields) = &mut result {
        fields.insert(
            "connection".into(),
            Value::object([("name", Value::string("GitHub"))]),
        );
    }
    let presentation = Presentation::default();
    let tool = Tool {
        name: "mcp",
        arguments: &args,
        result: Some(&result),
        summary: "Created #7",
        change: None,
        presentation: &presentation,
    };
    assert!(chapter_groupable(&tool));
    for width in [44, 100] {
        let header = chapter_header(&[tool], width, false);
        assert!(plain(&header.rows).contains("1 MCP call"));
        let rows = chapter_exception(&tool, width);
        let value = plain(&rows);
        assert!(value.contains("MCP GitHub"), "{value}");
        assert!(value.contains("Created #7"), "{value}");
        assert!(rows.iter().all(|row| text::cells(&row.plain()) <= width));
    }
}

#[test]
fn mcp_remote_text_cannot_emit_terminal_control_sequences() {
    let args = Value::object([
        (
            "connection_name",
            Value::string("GitHub\x1b]52;fixture\x07\nfake-row"),
        ),
        ("tool", Value::string("create_issue")),
    ]);
    let result = Value::object([(
        "result",
        Value::object([(
            "content",
            Value::Array(vec![Value::object([
                ("type", Value::string("text")),
                ("text", Value::string("Done\x1b]52;fixture\x07")),
            ])]),
        )]),
    )]);
    let presentation = Presentation::default();
    let tool = Tool {
        name: "mcp",
        arguments: &args,
        summary: "Done",
        result: Some(&result),
        change: None,
        presentation: &presentation,
    };
    for width in [44, 100] {
        let mut rows = draw("mcp", &args, Some(&result), None, width, &presentation);
        rows.extend(chapter_exception(&tool, width));
        assert!(
            rows.iter()
                .all(|row| !row.plain().chars().any(char::is_control))
        );
    }
}
