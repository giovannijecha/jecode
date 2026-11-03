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
    let tool = Tool {
        name: "lsp",
        arguments: &arguments,
        summary: "",
        result: Some(&result),
        change: None,
        presentation: &presentation,
    };
    assert!(Status::of(&tool) == Status::Done);
    let card = card(tool, 80, false);
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

fn last_row(arguments: &Value, result: &Value) -> Line {
    let presentation = Presentation::default();
    let card = card(
        Tool {
            name: "lsp",
            arguments,
            summary: "",
            result: Some(result),
            change: None,
            presentation: &presentation,
        },
        80,
        false,
    );
    card.rows.last().unwrap().clone()
}

#[test]
fn failed_call_shows_its_error_instead_of_an_empty_result() {
    let arguments = Value::object([("action", Value::string("definition"))]);
    let result = Value::object([
        ("status", Value::string("failed")),
        (
            "error",
            Value::string("LSP configuration must contain a servers array"),
        ),
        ("complete", Value::Bool(false)),
    ]);
    let row = last_row(&arguments, &result);
    assert_eq!(
        row.plain().trim_start_matches(['└', ' ']),
        "LSP configuration must contain a servers array"
    );
    assert!(row.spans.iter().any(|span| span.style == BAD));
}

#[test]
fn server_rpc_errors_fail_without_claiming_an_empty_result() {
    let detail = "LSP server error -32801: content modified (retrying may succeed)";
    for action in ["definition", "references", "diagnostics"] {
        let arguments = Value::object([("action", Value::string(action))]);
        let result = Value::object([
            ("status", Value::string("server_error")),
            ("detail", Value::string(detail)),
            ("locations", Value::Array(vec![])),
            ("diagnostics", Value::Array(vec![])),
            ("complete", Value::Bool(false)),
        ]);
        let presentation = Presentation::default();
        let tool = Tool {
            name: "lsp",
            arguments: &arguments,
            summary: "",
            result: Some(&result),
            change: None,
            presentation: &presentation,
        };
        assert!(Status::of(&tool) == Status::Failed, "{action}");
        let row = card(tool, 160, false).rows.last().unwrap().plain();
        assert!(row.contains("Server error"), "{action}: {row}");
        assert!(row.contains(detail), "{action}: {row}");
        assert!(!row.contains("No locations"), "{action}: {row}");
        assert!(!row.contains("Diagnostics unavailable"), "{action}: {row}");
    }
}

#[test]
fn truncated_results_count_from_totals_and_never_claim_absent_findings() {
    let diagnostics = Value::object([("action", Value::string("diagnostics"))]);
    let envelope = Value::object([
        ("status", Value::string("ready")),
        ("freshness", Value::string("current")),
        ("total_diagnostics", Value::number(100)),
        ("total_errors", Value::number(25)),
        ("total_warnings", Value::number(50)),
        ("truncated", Value::Bool(true)),
    ]);
    let row = last_row(&diagnostics, &envelope).plain();
    assert!(row.contains("25 errors · 50 warnings · current"), "{row}");
    let references = Value::object([
        ("status", Value::string("ready")),
        ("total_locations", Value::number(120)),
        ("locations", Value::Array(vec![Value::Null; 40])),
    ]);
    assert_eq!(summary_for(&references, "references"), "120 locations");
    let bare = Value::object([
        ("status", Value::string("ready")),
        ("freshness", Value::string("current")),
    ]);
    let summary = summary_for(&bare, "diagnostics");
    assert!(!summary.contains("0 errors"), "{summary}");
    assert!(!last_row(&diagnostics, &bare).plain().contains("0 errors"));
    assert_eq!(summary_for(&bare, "references"), "Locations unavailable");
}
