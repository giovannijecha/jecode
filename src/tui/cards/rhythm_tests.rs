use super::*;
use crate::tui::theme::{
    BAD, BADGE_BAD, BADGE_IDLE, BADGE_SHELL, BADGE_TEXT, BODY, CODE_STRING, KEYWORD, LIT, Style,
    Surface,
};

fn object<const N: usize>(fields: [(&str, &str); N]) -> Value {
    Value::object(fields.map(|(key, value)| (key, Value::string(value))))
}
fn draw<'a>(
    name: &'a str,
    args: &'a Value,
    result: Option<&'a Value>,
    shown: &'a Presentation,
    columns: usize,
) -> Card {
    card(
        Tool {
            name,
            arguments: args,
            result,
            summary: "complete",
            change: None,
            presentation: shown,
        },
        columns,
        false,
    )
}
/// The title badge: its ink and surface.
fn badge(card: &Card) -> (Style, Option<Surface>) {
    let badge = &card.rows[0].spans[0];
    (badge.style, badge.background)
}
fn plain(card: &Card) -> String {
    card.rows
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn described_commands_use_shell_colours_two_rows_and_quiet_inline_result() {
    let command = format!("printf 'two  spaces' -- {}", "long-argument ".repeat(24));
    let args = object([
        ("command", &command),
        ("description", "Inspect the renderer"),
    ]);
    let result = Value::object([
        ("exit_code", Value::number(0)),
        ("stdout", Value::string("first\nsecond\nready\n")),
        (
            "checkpoint",
            object([("status", "none"), ("reason", "No changes")]),
        ),
    ]);
    for columns in [40, 80] {
        let card = draw(
            "bash",
            &args,
            Some(&result),
            &Presentation::default(),
            columns,
        );
        assert_eq!(card.rows[0].plain(), " BASH  Inspect the renderer");
        assert_eq!(badge(&card), (BADGE_TEXT.bold(), Some(BADGE_SHELL)));
        assert!(
            card.rows[1]
                .plain()
                .starts_with("  │ $ printf 'two  spaces'")
        );
        assert!(card.rows[2].plain().starts_with("  │ "));
        assert!(card.rows[2].plain().ends_with('…'));
        assert_eq!(card.rows[3].plain(), "  └ ready  +2 lines · no checkpoint");
        assert_eq!(card.rows.len(), 4);
        assert!(card.hidden);
        assert!(
            card.rows[0]
                .spans
                .iter()
                .any(|span| span.text.ends_with("Inspect the renderer") && span.style == BODY)
        );
        assert!(
            card.rows[1]
                .spans
                .iter()
                .any(|span| span.style == CODE_STRING && span.background.is_none())
        );
        assert!(
            card.rows
                .iter()
                .all(|line| line.background.is_none() && text::cells(&line.plain()) <= columns)
        );
    }
}

#[test]
fn blank_description_uses_body_target_and_background_process_keeps_ampersand() {
    let result = Value::object([("exit_code", Value::number(0))]);
    for description in ["", "  \n "] {
        let args = object([("command", "cargo test"), ("description", description)]);
        let card = draw("bash", &args, Some(&result), &Presentation::default(), 80);
        assert_eq!(plain(&card), " BASH  cargo test\n  └ Done");
        assert_eq!(badge(&card), (BADGE_TEXT.bold(), Some(BADGE_SHELL)));
        assert_eq!(card.rows[0].spans.last().unwrap().style, BODY);
    }
    let args = object([
        ("action", "poll"),
        ("process_id", "p1"),
        ("description", "Serve preview"),
    ]);
    let result = object([
        ("status", "running"),
        ("process_id", "p1"),
        ("command", "node preview.js"),
    ]);
    let card = draw(
        "process",
        &args,
        Some(&result),
        &Presentation::default(),
        80,
    );
    // The background call ends its title target in a bold `&`.
    assert_eq!(card.rows[0].plain(), " PROCESS  Serve preview &");
    assert_eq!(card.rows[0].spans.last().unwrap().text, " &");
    assert_eq!(card.rows[0].spans.last().unwrap().style, LIT.bold());
    assert_eq!(badge(&card), (LIT.bold(), Some(BADGE_IDLE)));
    assert_eq!(card.rows[1].plain(), "  │ $ node preview.js");
    // Background facts stay quiet; the badge carries the state.
    assert!(card.rows[2].plain().starts_with("  └ "));
    assert!(
        card.rows[2]
            .spans
            .iter()
            .skip(1)
            .all(|span| span.style == DIM)
    );
    let done = Value::object([("exit_code", Value::number(0))]);
    let finished = draw("process", &args, Some(&done), &Presentation::default(), 80);
    assert!(!finished.rows[0].plain().contains('&'));
}

#[test]
fn browser_decodes_cdp_exception_and_chapter_shows_one_exception_row() {
    let args = object([
        ("action", "evaluate"),
        ("expression", "const x = document.title; x"),
        ("description", "Read title"),
    ]);
    let result = object([(
        "error",
        "Browser JavaScript error: {\"text\":\"Uncaught\",\"exception\":{\"description\":\"TypeError: missing title\\n    at <anonymous>:1:10\\n    at ignored:2:3\"}}",
    )]);
    let shown = Presentation::default();
    let card = draw("browser", &args, Some(&result), &shown, 80);
    assert_eq!(card.rows[0].plain(), " BROWSER  evaluate");
    assert_eq!(badge(&card), (BADGE_TEXT.bold(), Some(BADGE_BAD)));
    assert!(
        card.rows[1]
            .spans
            .iter()
            .any(|span| span.style == KEYWORD && span.text == "const")
    );
    assert!(plain(&card).contains("TypeError: missing title · at <anonymous>:1:10"));
    assert!(!plain(&card).contains("exception") && !plain(&card).contains("ignored"));
    assert!(
        card.rows[2]
            .spans
            .iter()
            .any(|span| span.style == BAD && span.text.contains("TypeError"))
    );
    assert!(
        card.rows[2]
            .spans
            .iter()
            .any(|span| span.style == MUTED && span.text.contains("at <anonymous>"))
    );
    let rows = chapter_exception(
        &Tool {
            name: "browser",
            arguments: &args,
            result: Some(&result),
            summary: "",
            change: None,
            presentation: &shown,
        },
        80,
    );
    assert_eq!(rows[0].plain(), "  ⎿ BROWSER evaluate Read title");
    assert!(rows[1].plain().starts_with("    TypeError"));
}

#[test]
fn expanded_output_and_checkpoint_notice_remain_complete() {
    let args = object([
        ("command", "printf output"),
        ("description", "Print output"),
    ]);
    let result = Value::object([
        ("exit_code", Value::number(0)),
        ("stdout", Value::string("first\n\nlast\n")),
        (
            "checkpoint",
            object([("status", "none"), ("reason", "No changes")]),
        ),
    ]);
    let shown = Presentation {
        expanded: true,
        ..Presentation::default()
    };
    let card = draw("bash", &args, Some(&result), &shown, 40);
    assert!(plain(&card).contains("  │ first\n  │ \n  │ last"));
    assert!(plain(&card).contains("Checkpoint none · No changes"));
    assert!(!card.hidden);
}

#[test]
fn cdp_stack_frames_supply_a_muted_position_at_both_widths() {
    let args = object([
        ("action", "evaluate"),
        ("expression", "throw Error('boom')"),
    ]);
    let result = object([(
        "error",
        "Browser JavaScript error: {\"text\":\"Uncaught\",\"exception\":{\"description\":\"Error: boom\"},\"stackTrace\":{\"callFrames\":[{\"url\":\"app.js\",\"lineNumber\":2,\"columnNumber\":3}]}}",
    )]);
    for columns in [40, 80] {
        let card = draw(
            "browser",
            &args,
            Some(&result),
            &Presentation::default(),
            columns,
        );
        assert!(plain(&card).contains("Error: boom · at app.js:3:4"));
        assert!(
            card.rows
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| span.style == MUTED && span.text.contains("at app.js:3:4"))
        );
        assert!(!plain(&card).contains("stackTrace"));
    }
}

#[test]
fn long_commands_and_failures_keep_one_ellipsis_and_two_extra_stderr_rows() {
    let args = object([("command", &"x".repeat(300)), ("description", "Run check")]);
    let result = Value::object([
        ("exit_code", Value::number(2)),
        (
            "stderr",
            Value::string(format!(
                "{}\nextra one\nextra two\nextra three\n",
                "error ".repeat(40)
            )),
        ),
    ]);
    for columns in [40, 80] {
        let card = draw(
            "bash",
            &args,
            Some(&result),
            &Presentation::default(),
            columns,
        );
        assert!(card.rows[2].plain().ends_with('…'));
        assert!(!card.rows[2].plain().ends_with("……"));
        assert!(card.rows[4].plain().ends_with('…'));
        assert!(plain(&card).contains("extra one") && plain(&card).contains("extra two"));
        assert!(!plain(&card).contains("extra three"));
        assert!(
            card.rows
                .iter()
                .all(|line| text::cells(&line.plain()) <= columns)
        );
    }
}

#[test]
fn exec_cards_keep_structured_target_and_command_status_across_states() {
    let args = Value::object([
        ("executable", Value::string("cargo")),
        (
            "args",
            Value::Array(vec![Value::string("test"), Value::string("a b")]),
        ),
    ]);
    let collapsed = Presentation::default();
    let pending = draw("exec", &args, None, &collapsed, 80);
    assert!(
        pending.rows[0]
            .plain()
            .starts_with(" EXEC  cargo [\"test\",\"a b\"]")
    );
    let success = Value::object([
        ("exit_code", Value::number(0)),
        ("stdout", Value::string("ready\n")),
    ]);
    let done = draw("exec", &args, Some(&success), &collapsed, 80);
    assert!(plain(&done).contains("ready"));
    let failed = Value::object([
        ("exit_code", Value::number(7)),
        ("stderr", Value::string("failed step\n")),
    ]);
    let failed_card = draw("exec", &args, Some(&failed), &collapsed, 80);
    assert!(plain(&failed_card).contains("failed step"));
    assert_eq!(badge(&failed_card), (BADGE_TEXT.bold(), Some(BADGE_BAD)));
    let background = object([("status", "running"), ("process_id", "process-fixture")]);
    let running = draw("exec", &args, Some(&background), &collapsed, 80);
    assert!(plain(&running).contains("process-fixture"));
    let expanded = Presentation {
        expanded: true,
        ..Presentation::default()
    };
    let open = draw("exec", &args, Some(&failed), &expanded, 80);
    assert!(plain(&open).contains("failed step"));
    assert!(plain(&open).contains("exit 7"));
    let described = Value::object([
        ("executable", Value::string("cargo")),
        ("args", Value::Array(vec![Value::string("test")])),
        ("description", Value::string("Run checks")),
    ]);
    let card = draw("exec", &described, Some(&success), &collapsed, 80);
    assert_eq!(card.rows[0].plain(), " EXEC  Run checks");
    assert!(card.rows[1].plain().contains("> cargo [\"test\"]"));
}

#[test]
fn browser_actions_without_a_payload_open_their_result_on_the_first_tree_row() {
    let result = Value::object([
        ("title", Value::string("Example Domain")),
        ("url", Value::string("https://example.com/")),
        ("text", Value::string("{\n  \"elements\": []\n}")),
    ]);
    for action in ["snapshot", "stop"] {
        let args = object([("action", action)]);
        for expanded in [false, true] {
            let shown = Presentation {
                expanded,
                ..Presentation::default()
            };
            let card = draw("browser", &args, Some(&result), &shown, 80);
            assert_eq!(card.rows[0].plain(), format!(" BROWSER  {action}"));
            // No empty command row opens the tree; this result has no blank line.
            let empty = |row: &Line| row.plain().trim_start_matches([' ', '│', '└']).is_empty();
            assert!(!card.rows[1..].iter().any(empty), "{}", plain(&card));
        }
    }
    // A payload still shows as the command on the tree.
    let args = object([("action", "open"), ("url", "https://example.com/")]);
    let card = draw(
        "browser",
        &args,
        Some(&result),
        &Presentation::default(),
        80,
    );
    assert!(card.rows[1].plain().contains("https://example.com/"));
}
