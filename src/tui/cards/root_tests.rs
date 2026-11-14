use super::*;
use crate::tui::theme::{BAD, MUTED};

fn receipt(code: Value, descendants: Option<usize>) -> Value {
    let mut fields = vec![
        ("status".into(), Value::string("running")),
        ("process_id".into(), Value::string("process-7")),
        (
            "root_process".into(),
            Value::object([
                ("pid", Value::number(701)),
                ("status", Value::string("exited")),
                ("exit_code", code),
            ]),
        ),
    ];
    if let Some(count) = descendants {
        fields.push((
            "descendants".into(),
            Value::object([("running", Value::number(count))]),
        ));
    } else {
        fields.push((
            "waiting_for".into(),
            Value::string("command output held open"),
        ));
    }
    Value::Object(fields.into_iter().collect())
}

fn rendered(name: &str, result: &Value, expanded: bool) -> Card {
    let args = Value::object([
        ("action", Value::string("poll")),
        ("id", Value::string("process-7")),
    ]);
    let presentation = Presentation {
        expanded,
        ..Presentation::default()
    };
    card(
        Tool {
            name,
            arguments: &args,
            summary: "poll complete",
            result: Some(result),
            change: None,
            presentation: &presentation,
        },
        80,
        false,
    )
}

#[test]
fn active_process_receipts_name_root_outcome_and_open_boundary() {
    for (code, descendants, expected, style) in [
        (
            Value::number(0),
            Some(2),
            "Root exited 0 · 2 descendants running",
            MUTED,
        ),
        (
            Value::number(7),
            Some(2),
            "Root exited 7 · 2 descendants running",
            BAD,
        ),
        (
            Value::number(7),
            None,
            "Root exited 7 · output still open",
            BAD,
        ),
        (
            Value::Null,
            None,
            "Root exited without code · output still open",
            BAD,
        ),
    ] {
        let result = receipt(code, descendants);
        for expanded in [false, true] {
            let card = rendered("process", &result, expanded);
            let rows = card
                .rows
                .iter()
                .map(Line::plain)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(rows.contains(expected), "{expanded}: {rows}");
            assert!(
                card.rows
                    .iter()
                    .all(|line| text::cells(&line.plain()) <= 80)
            );
            assert!(card.rows.iter().any(|line| {
                line.spans
                    .iter()
                    .any(|span| span.style == style && span.text.contains("Root exited"))
            }));
        }
    }
}

#[test]
fn launching_calls_show_the_root_outcome_before_the_boundary_ends() {
    let result = receipt(Value::number(7), Some(1));
    for name in ["bash", "exec"] {
        for expanded in [false, true] {
            let shown = rendered(name, &result, expanded)
                .rows
                .iter()
                .map(Line::plain)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                shown.contains("Root exited 7 · 1 descendant running"),
                "{name}: {shown}"
            );
            assert!(!shown.contains("Done"), "{shown}");
        }
    }
}

#[test]
fn compact_work_shows_root_fact_while_final_receipt_has_its_own_outcome() {
    let args = Value::object([
        ("action", Value::string("poll")),
        ("work", Value::string("Check server")),
    ]);
    let mut active = receipt(Value::number(7), Some(2));
    if let Value::Object(fields) = &mut active {
        fields.insert("type".into(), Value::string("process_root_exited"));
    }
    let final_result = Value::object([
        ("status", Value::string("exited")),
        ("process_id", Value::string("process-7")),
        ("exit_code", Value::number(7)),
        (
            "root_process",
            Value::object([
                ("status", Value::string("exited")),
                ("exit_code", Value::number(7)),
            ]),
        ),
    ]);
    let shown = Presentation::default();
    let tool = |result| Tool {
        name: "process",
        arguments: &args,
        summary: "complete",
        result: Some(result),
        change: None,
        presentation: &shown,
    };
    let active_tool = tool(&active);
    assert!(Status::of(&active_tool) == Status::Background);
    let final_tool = tool(&final_result);
    let card = chapter::header(&[active_tool, final_tool], 80, false);
    let rows = card
        .rows
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        rows.contains("Root exited 7 · 2 descendants running"),
        "{rows}"
    );
    assert!(rows.contains("exit 7"), "{rows}");
    assert_eq!(rows.matches("Root exited").count(), 1, "{rows}");
}
