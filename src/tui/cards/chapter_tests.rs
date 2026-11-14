#[path = "chapter_concurrency_tests.rs"]
mod concurrency;
#[path = "chapter_stability_tests.rs"]
mod stability;

use super::*;
use crate::tui::theme::{BAD, DIM, STRONG, WARNING};
use std::time::Duration;

fn object<const N: usize>(fields: [(&str, &str); N]) -> Value {
    Value::object(fields.map(|(key, value)| (key, Value::string(value))))
}

fn tool<'a>(
    name: &'a str,
    arguments: &'a Value,
    result: Option<&'a Value>,
    presentation: &'a Presentation,
) -> Tool<'a> {
    Tool {
        name,
        arguments,
        summary: "complete",
        result,
        change: None,
        presentation,
    }
}

fn lines(card: &Card) -> Vec<String> {
    card.rows.iter().map(Line::plain).collect()
}

#[test]
fn labels_are_trimmed_and_skill_reads_stay_outside_chapters() {
    let shown = Presentation::default();
    let read = object([("path", "src/lib.rs"), ("work", " Inspect source ")]);
    assert_eq!(
        chapter::label(&tool("read", &read, None, &shown)),
        "Inspect source"
    );
    let skill = object([("path", "skill:review/SKILL.md")]);
    assert!(!chapter::groupable(&tool("read", &skill, None, &shown)));
    for name in [
        "read", "list", "search", "bash", "exec", "process", "web", "browser", "mcp", "lsp",
    ] {
        assert!(chapter::groupable(&tool(name, &read, None, &shown)));
    }
    assert!(!chapter::groupable(&tool("edit", &read, None, &shown)));
}

#[test]
fn compact_has_at_most_eight_physical_rows_at_all_study_widths() {
    let shown = Presentation {
        elapsed: Some(Duration::from_secs(5)),
        ..Presentation::default()
    };
    let args: Vec<_> = (0..96)
        .map(|index| {
            object([
                ("path", &format!("src/long/path/to/file-{index}.rs")),
                ("work", "Inspect"),
            ])
        })
        .collect();
    let result = Value::object([]);
    let calls: Vec<_> = args
        .iter()
        .map(|args| tool("read", args, Some(&result), &shown))
        .collect();
    for columns in [40, 80, 120] {
        let card = chapter::header(&calls, columns, true);
        let rows = lines(&card);
        assert!(rows.len() <= 8, "{columns}: {rows:?}");
        assert!(
            rows.iter().all(|line| text::cells(line) <= columns),
            "{rows:?}"
        );
        assert!(rows[0].contains("WORK") && rows[0].contains("96 calls"));
        assert!(!rows[0].contains("Ctrl+O"));
        assert!(rows.last().unwrap().contains("92 calls folded"), "{rows:?}");
        assert!(rows.last().unwrap().ends_with("Ctrl+O expand"));
        assert!(card.hidden);
        assert!(card.rows[0].origin.is_some_and(|origin| origin.line == 0));
    }
}

#[test]
fn long_facts_use_two_rows_and_show_the_folded_ellipsis() {
    let shown = Presentation::default();
    let result = Value::object([]);
    let names = [
        "read", "list", "search", "web", "browser", "bash", "process", "mcp", "lsp", "agent",
    ];
    let args: Vec<_> = names
        .iter()
        .map(|_| object([("path", "src/example.rs"), ("action", "screenshot")]))
        .collect();
    let calls: Vec<_> = names
        .iter()
        .zip(&args)
        .map(|(name, args)| tool(name, args, Some(&result), &shown))
        .collect();
    let card = chapter::header(&calls, 40, false);
    let rows = lines(&card);
    assert!(rows.len() <= 8, "{rows:?}");
    assert!(rows[1].starts_with("  │ "));
    assert!(
        rows[2].starts_with("  │ ") && rows[2].ends_with(" …"),
        "{rows:?}"
    );
    assert_eq!(rows[2].matches('…').count(), 1, "{rows:?}");
    assert!(rows.iter().all(|row| text::cells(row) <= 40));
}

#[test]
fn priority_overflow_precedes_recent_done_across_families() {
    let shown = Presentation::default();
    let args: Vec<_> = (0..10)
        .map(|index| object([("command", &format!("run-{index}")), ("work", "Inspect")]))
        .collect();
    let done = Value::object([("exit_code", Value::number(0))]);
    let fail = Value::object([
        ("exit_code", Value::number(7)),
        ("stderr", Value::string("diagnostic secret")),
    ]);
    let stopped = Value::object([("cancelled", Value::Bool(true))]);
    let unknown = Value::object([("outcome", Value::string("unknown"))]);
    let mcp_args = object([("description", "run-3"), ("work", "Inspect")]);
    let calls = [
        tool("bash", &args[0], Some(&done), &shown),
        tool("bash", &args[1], Some(&fail), &shown),
        tool("bash", &args[2], Some(&stopped), &shown),
        tool("mcp", &mcp_args, Some(&unknown), &shown),
        tool("bash", &args[4], None, &shown),
        tool("bash", &args[5], Some(&fail), &shown),
        tool("bash", &args[6], Some(&done), &shown),
        tool("exec", &args[7], Some(&done), &shown),
        tool("bash", &args[8], Some(&done), &shown),
        tool("bash", &args[9], Some(&done), &shown),
    ];
    let card = chapter::header(&calls, 80, false);
    let rows = lines(&card);
    let snapshot = rows.join("\n");
    assert!(rows.len() <= 8, "{rows:?}");
    assert!(
        snapshot.contains("run-1") && snapshot.contains("run-3") && snapshot.contains("run-5"),
        "{snapshot}"
    );
    assert!(
        !snapshot.contains("run-2") && !snapshot.contains("run-4"),
        "{snapshot}"
    );
    assert!(
        snapshot.contains("+2 more calls need attention"),
        "{snapshot}"
    );
    assert!(!snapshot.contains("diagnostic secret"), "{snapshot}");
    assert_eq!(snapshot.matches("exit 7").count(), 2);
    assert!(
        card.rows
            .iter()
            .flat_map(|row| &row.spans)
            .any(|span| span.text.contains("attention") && span.style == WARNING)
    );

    let old = object([("path", "old.rs")]);
    let new = Value::object([
        ("executable", Value::string("newer")),
        ("args", Value::Array(vec![])),
    ]);
    let recent = [
        tool("read", &old, Some(&done), &shown),
        tool("exec", &new, Some(&done), &shown),
    ];
    let recent_text = lines(&chapter::header(&recent, 80, false)).join("\n");
    assert!(recent_text.contains("EXEC newer"), "{recent_text}");
}

#[test]
fn equal_priority_uses_latest_three_then_restores_source_order() {
    let shown = Presentation::default();
    let failed = Value::object([("exit_code", Value::number(2))]);
    let args: Vec<_> = (0..6)
        .map(|index| object([("command", &format!("call-{index}"))]))
        .collect();
    let calls: Vec<_> = args
        .iter()
        .map(|args| tool("bash", args, Some(&failed), &shown))
        .collect();
    let rows = lines(&chapter::header(&calls, 80, false));
    let text = rows.join("\n");
    assert!(!text.contains("call-0") && !text.contains("call-1") && !text.contains("call-2"));
    let selected = ["call-3", "call-4", "call-5"];
    let positions: Vec<_> = selected
        .iter()
        .map(|name| text.find(name).unwrap())
        .collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
    assert!(text.contains("+3 more calls need attention"), "{text}");
}

#[test]
fn title_member_and_footer_styles_follow_proposal() {
    let args = object([
        (
            "command",
            "printf a very long target that must be shortened in the middle to keep the tail.rs",
        ),
        ("work", "Inspect parser"),
    ]);
    let shown = Presentation {
        elapsed: Some(Duration::from_secs(7)),
        ..Presentation::default()
    };
    let failed = Value::object([("exit_code", Value::number(101))]);
    let calls = [tool("bash", &args, Some(&failed), &shown); 2];
    let card = chapter::header(&calls, 40, false);
    let rows = lines(&card);
    assert_eq!(text::cells(&rows[0]), 38);
    assert!(rows[0].contains("2 calls · 2 failed"));
    assert!(
        card.rows[0]
            .spans
            .iter()
            .any(|span| span.text.contains("Inspect") && span.style == STRONG.bold())
    );
    assert!(
        card.rows[0]
            .spans
            .iter()
            .any(|span| span.text.contains("failed") && span.style == BAD)
    );
    assert!(
        rows.iter()
            .any(|row| row.contains('…') && row.contains("exit 101")),
        "{rows:?}"
    );
    assert!(
        card.rows
            .iter()
            .flat_map(|row| &row.spans)
            .any(|span| span.text.contains("BASH") && span.background.is_none())
    );
    assert!(
        card.rows
            .iter()
            .flat_map(|row| &row.spans)
            .any(|span| span.text.contains("calls folded") && span.style == DIM)
    );
}

#[test]
fn failed_members_only_show_real_exit_codes() {
    let shown = Presentation::default();
    let done = Value::object([]);
    let read_args = object([("path", "src/done.rs"), ("work", "Inspect")]);
    let read = tool("read", &read_args, Some(&done), &shown);
    let mcp_args = object([
        ("connection", "source"),
        ("tool", "inspect"),
        ("work", "Inspect"),
    ]);
    let mcp_failure = Value::object([("error", Value::string("mcp diagnostic"))]);
    let lsp_args = object([
        ("server", "rust"),
        ("action", "references"),
        ("work", "Inspect"),
    ]);
    let lsp_failure = object([("status", "failed"), ("message", "lsp diagnostic")]);
    let agent_args = object([("label", "Agent 1"), ("work", "Inspect")]);
    let agent_failure = Value::object([
        ("kind", Value::string("returned")),
        (
            "details",
            Value::Array(vec![Value::string("agent diagnostic")]),
        ),
    ]);
    for (call, name, diagnostic) in [
        (
            tool("mcp", &mcp_args, Some(&mcp_failure), &shown),
            "MCP CALL",
            "mcp diagnostic",
        ),
        (
            tool("lsp", &lsp_args, Some(&lsp_failure), &shown),
            "LSP CALL",
            "lsp diagnostic",
        ),
        (
            tool("collaboration", &agent_args, Some(&agent_failure), &shown),
            "AGENT 1",
            "agent diagnostic",
        ),
    ] {
        let card = chapter::header(&[call, read], 80, false);
        let member = card
            .rows
            .iter()
            .find(|row| row.plain().contains(name))
            .unwrap();
        assert!(
            member.plain().trim_end().ends_with("failed"),
            "{name}: {:?}",
            lines(&card)
        );
        assert!(
            member
                .spans
                .iter()
                .any(|span| span.text == "failed" && span.style == BAD)
        );
        let snapshot = lines(&card).join("\n");
        assert!(
            !snapshot.contains("exit 1") && !snapshot.contains(diagnostic),
            "{snapshot}"
        );
    }

    let bash_args = object([("command", "cargo test"), ("work", "Inspect")]);
    let bash_failure = Value::object([("exit_code", Value::number(101))]);
    let card = chapter::header(
        &[tool("bash", &bash_args, Some(&bash_failure), &shown), read],
        80,
        false,
    );
    let member = card
        .rows
        .iter()
        .find(|row| row.plain().contains("BASH"))
        .unwrap();
    assert!(member.plain().trim_end().ends_with("exit 101"));
    assert!(
        member
            .spans
            .iter()
            .any(|span| span.text == "exit 101" && span.style == BAD)
    );
}

#[test]
fn mixed_outcome_facts_follow_background_stopped_running_order() {
    let shown = Presentation::default();
    let args = object([("command", "run"), ("work", "Inspect")]);
    let background = object([("status", "running"), ("process_id", "p1")]);
    let stopped = Value::object([("cancelled", Value::Bool(true))]);
    let calls = [
        tool("bash", &args, None, &shown),
        tool("bash", &args, Some(&background), &shown),
        tool("bash", &args, Some(&stopped), &shown),
    ];
    let snapshot = lines(&chapter::header(&calls, 80, false)).join("\n");
    assert!(
        snapshot.contains("1 in background · 1 stopped · 1 running"),
        "{snapshot}"
    );
}

#[test]
fn live_chapter_has_no_key_and_failed_member_has_no_diagnostic_subrow() {
    let args = object([("command", "run"), ("work", "Inspect")]);
    let shown = Presentation::default();
    let failed = Value::object([
        ("exit_code", Value::number(9)),
        ("stderr", Value::string("first\nsecond")),
    ]);
    let calls = [
        tool("bash", &args, Some(&failed), &shown),
        tool("bash", &args, None, &shown),
    ];
    let card = chapter::header(&calls, 40, false);
    let text = lines(&card).join("\n");
    assert!(
        !text.contains("Ctrl+O") && !text.contains("first") && !text.contains("second"),
        "{text}"
    );
    assert_eq!(text.matches("exit 9").count(), 1);
    assert!(text.contains("Running · 0.0s"), "{text}");
    let ended_with_unresolved_call = chapter::header(&calls, 40, true);
    assert!(
        ended_with_unresolved_call
            .rows
            .last()
            .unwrap()
            .plain()
            .ends_with("Ctrl+O expand")
    );
}

#[test]
fn multiline_descriptions_stay_on_one_physical_preview_row() {
    let shown = Presentation {
        elapsed: Some(Duration::ZERO),
        ..Presentation::default()
    };
    let args = object([
        ("command", "inspect"),
        ("description", "Inspect\n  source\tfiles"),
    ]);
    let result = Value::object([("exit_code", Value::number(0))]);
    let calls = [
        tool("bash", &args, Some(&result), &shown),
        tool("bash", &args, Some(&result), &shown),
    ];
    for columns in [40, 80, 120] {
        let card = chapter::header(&calls, columns, true);
        assert!(card.rows.len() <= 8);
        assert!(
            card.rows
                .iter()
                .all(|row| !row.plain().contains(['\n', '\r', '\t']))
        );
        assert!(
            card.rows
                .iter()
                .any(|row| row.plain().contains("Inspect source"))
        );
    }
}
