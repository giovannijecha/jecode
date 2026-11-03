use super::*;
use crate::tui::{
    cards::Presentation,
    theme::{BAD, BADGE_IDLE, WARNING},
};

#[test]
fn narrow_card_keeps_action_and_expanded_arguments_wrap() {
    let arguments = Value::object([
        ("action", Value::string("references")),
        ("server", Value::string("lsp-a-very-long-stable-identifier")),
        (
            "path",
            Value::string("C:/A very long project name/source/main.rs"),
        ),
        ("line", Value::number(12)),
        ("column", Value::number(4)),
    ]);
    let result = Value::object([
        ("status", Value::string("ready")),
        ("locations", Value::Array(vec![])),
        ("complete", Value::Bool(true)),
    ]);
    let presentation = Presentation::default();
    for width in [44, 80, 120] {
        let tool = Tool {
            name: "lsp",
            arguments: &arguments,
            summary: "",
            result: Some(&result),
            change: None,
            presentation: &presentation,
        };
        let compact = card(tool, width, true);
        assert!(compact.rows[0].plain().contains("references"));
        assert!(compact.rows[0].plain().contains("Ctrl+O"));
        assert!(
            compact
                .rows
                .iter()
                .all(|row| text::cells(&row.plain()) <= width)
        );
        let expanded = Presentation {
            expanded: true,
            ..Presentation::default()
        };
        let card = card(
            Tool {
                presentation: &expanded,
                ..tool
            },
            width,
            true,
        );
        assert!(card.rows[0].plain().contains("references"));
        assert!(card.rows[0].plain().contains("Ctrl+O"));
        assert!(
            card.rows
                .iter()
                .any(|row| row.plain().contains("lsp-a-very-long"))
        );
        assert!(
            card.rows
                .iter()
                .all(|row| text::cells(&row.plain()) <= width)
        );
    }
}

#[test]
fn diagnostic_findings_are_successful_with_colored_counts() {
    let arguments = Value::object([("action", Value::string("diagnostics"))]);
    let result = Value::object([
        ("status", Value::string("ready")),
        ("freshness", Value::string("current")),
        (
            "diagnostics",
            Value::Array(vec![
                Value::object([("severity", Value::number(1))]),
                Value::object([("severity", Value::number(2))]),
            ]),
        ),
    ]);
    let presentation = Presentation::default();
    let card = card(
        Tool {
            name: "lsp",
            arguments: &arguments,
            summary: "",
            result: Some(&result),
            change: None,
            presentation: &presentation,
        },
        80,
        false,
    );
    assert_eq!(card.rows[0].spans[0].background, Some(BADGE_IDLE));
    assert!(
        card.rows
            .last()
            .unwrap()
            .spans
            .iter()
            .any(|span| span.style == BAD)
    );
    assert!(
        card.rows
            .last()
            .unwrap()
            .spans
            .iter()
            .any(|span| span.style == WARNING)
    );
    assert!(card.rows.last().unwrap().plain().contains("current"));
}

#[test]
fn only_current_empty_diagnostics_claim_zero_findings() {
    for (freshness, expected) in [
        ("current", "0 errors · 0 warnings · current"),
        ("pending", "Diagnostics pending"),
        ("stale", "Stale diagnostics"),
        ("unverified", "Unverified version"),
        ("unavailable", "Diagnostics unavailable"),
        ("sync_gap", "Sync incomplete"),
    ] {
        let result = Value::object([
            ("status", Value::string("ready")),
            ("freshness", Value::string(freshness)),
            ("diagnostics", Value::Array(vec![])),
        ]);
        let summary = summary_for(&result, "diagnostics");
        assert!(summary.contains(expected), "{summary}");
        if freshness != "current" {
            assert!(!summary.contains("0 errors"), "{summary}");
        }
    }
}
