use std::time::Duration;

use super::*;
use crate::tui::theme::{DIFF_REMOVED_SURFACE, TOOL_SELECTED_SURFACE};

fn draw<'a>(
    name: &'a str,
    arguments: &'a Value,
    result: Option<&'a Value>,
    columns: usize,
    last: bool,
    preview: bool,
    presentation: &'a Presentation,
) -> Vec<Line> {
    super::render(
        Tool {
            name,
            arguments,
            summary: "wrote 20 bytes",
            result,
            last,
            preview,
            presentation,
        },
        columns,
    )
}

fn command(value: &str) -> Value {
    Value::object([("command", Value::string(value))])
}

fn lines(rows: &[Line]) -> Vec<String> {
    rows.iter().map(Line::plain).collect()
}

#[test]
fn wide_status_is_right_aligned_and_preview_extent_fits_the_header() {
    let arguments = command("cargo test");
    let result = Value::object([
        ("stdout", Value::string("earlier\nfirst\nsecond\nlast")),
        ("exit_code", Value::number(0)),
    ]);
    let presentation = Presentation {
        elapsed: Some(Duration::from_millis(8400)),
        ..Presentation::default()
    };
    let rows = draw(
        "bash",
        &arguments,
        Some(&result),
        90,
        false,
        true,
        &presentation,
    );
    let plain = lines(&rows);
    assert!(plain[0].starts_with("├─ bash  cargo test"));
    assert!(plain[0].ends_with("✓ exit 0  ·  8.4s · preview 3 / 4"));
    assert_eq!(text::cells(&plain[0]), 89);
    assert_eq!(&plain[1..], ["│    first", "│    second", "│    last"]);
    assert!(
        rows[0]
            .spans
            .iter()
            .any(|span| span.text == "bash  " && span.style == EMPHASIS.bold())
    );
    assert!(
        rows[0]
            .spans
            .iter()
            .any(|span| span.text.starts_with("✓ ") && span.style == GOOD)
    );
    assert_eq!(rows[0].spans[0].style, FAINT);
}

#[test]
fn narrow_header_wraps_the_target_and_moves_status_to_its_own_row() {
    let arguments = command("first second third fourth fifth sixth seventh");
    let result = Value::object([("exit_code", Value::number(0))]);
    let rows = draw(
        "bash",
        &arguments,
        Some(&result),
        30,
        true,
        true,
        &Presentation::default(),
    );
    assert_eq!(
        lines(&rows),
        [
            "└─ bash  first second third fo",
            "   urth fifth sixth seventh",
            "     ✓ exit 0",
        ]
    );
    assert!(rows.iter().all(|row| text::cells(&row.plain()) <= 30));
}

#[test]
fn status_changes_rows_at_the_catalog_boundary_and_keeps_short_durations() {
    let arguments = command("cargo test");
    let result = Value::object([("exit_code", Value::number(0))]);
    let presentation = Presentation {
        elapsed: Some(Duration::from_millis(100)),
        ..Presentation::default()
    };
    let narrow = draw(
        "bash",
        &arguments,
        Some(&result),
        71,
        true,
        true,
        &presentation,
    );
    let wide = draw(
        "bash",
        &arguments,
        Some(&result),
        72,
        true,
        true,
        &presentation,
    );
    assert_eq!(narrow.len(), 2);
    assert!(narrow[1].plain().starts_with("     ✓ exit 0  ·  0.1s"));
    assert_eq!(wide.len(), 1);
    assert!(wide[0].plain().ends_with("✓ exit 0  ·  0.1s"));
    assert_eq!(wide[0].origin.unwrap().line, 0);
}

#[test]
fn statuses_keep_separate_glyph_and_outcome_colors() {
    let arguments = command("fixture");
    let cases = [
        (None, "running", "", TOOL_ACCENT),
        (
            Some(Value::object([("exit_code", Value::number(0))])),
            "exit 0",
            "✓ ",
            GOOD,
        ),
        (
            Some(Value::object([("exit_code", Value::number(7))])),
            "exit 7",
            "× ",
            BAD,
        ),
        (
            Some(Value::object([("cancelled", Value::Bool(true))])),
            "cancelled",
            "■ ",
            WARNING,
        ),
        (
            Some(Value::object([("timed_out", Value::Bool(true))])),
            "timed out",
            "× ",
            BAD,
        ),
    ];
    for (result, outcome, glyph, style) in cases {
        let rows = draw(
            "bash",
            &arguments,
            result.as_ref(),
            90,
            true,
            true,
            &Presentation::default(),
        );
        let header = &rows[0];
        assert!(header.plain().ends_with(&format!("{glyph}{outcome}")));
        assert!(
            header
                .spans
                .iter()
                .any(|span| span.text.contains(outcome) && span.style == style)
        );
    }
}

#[test]
fn failed_preview_prioritizes_stderr_then_recent_stdout() {
    let arguments = command("cargo build");
    let result = Value::object([
        ("stdout", Value::string("earlier\nlatest")),
        ("stderr", Value::string("first error\nsecond error")),
        ("exit_code", Value::number(101)),
    ]);
    let rows = draw(
        "bash",
        &arguments,
        Some(&result),
        90,
        true,
        true,
        &Presentation::default(),
    );
    let plain = lines(&rows);
    assert!(plain[0].ends_with("× exit 101 · preview 3 / 4"));
    assert_eq!(
        &plain[1..],
        [
            "     stderr: first error",
            "     second error",
            "     latest"
        ]
    );
    assert!(!plain.iter().any(|row| row.contains("earlier")));
    assert!(rows[1].spans.iter().any(|span| span.style == BAD));
}

#[test]
fn retained_tail_and_capture_notice_are_distinct() {
    let arguments = command("fixture");
    let result = Value::object([
        (
            "stdout",
            Value::string("first\nsecond\nthird\nfourth\nlast"),
        ),
        ("exit_code", Value::number(0)),
        ("stdout_bytes", Value::number(70000)),
        ("stdout_truncated", Value::Bool(true)),
        ("output_limit_bytes", Value::number(65536)),
    ]);
    let rows = draw(
        "bash",
        &arguments,
        Some(&result),
        100,
        true,
        true,
        &Presentation::default(),
    );
    let plain = lines(&rows).join("\n");
    assert!(rows[0].plain().ends_with("✓ exit 0 · preview 3 / 5"));
    assert!(plain.contains("third\n     fourth\n     last"));
    assert!(plain.contains("capture truncated: 65536 of 70000 bytes retained"));
    assert!(!plain.contains("first\n"));
    assert_eq!(
        result.get("stdout").and_then(Value::as_str),
        Some("first\nsecond\nthird\nfourth\nlast")
    );
}

#[test]
fn read_preview_keeps_first_rows_and_next_offset_notice() {
    let arguments = Value::object([
        ("path", Value::string("fixture.rs")),
        ("offset", Value::number(4)),
    ]);
    let result = Value::object([
        (
            "content",
            Value::string("4: one\n5: two\n6: three\n7: four"),
        ),
        ("lines_returned", Value::number(4)),
        ("next_offset", Value::number(8)),
    ]);
    let rows = draw(
        "read",
        &arguments,
        Some(&result),
        100,
        true,
        true,
        &Presentation::default(),
    );
    let plain = lines(&rows).join("\n");
    assert!(rows[0].plain().ends_with("✓ lines 4–7 · preview 3 / 4"));
    assert!(plain.contains("4: one"));
    assert!(!plain.contains("7: four"));
    assert!(plain.contains("More file lines available from line 8"));
    assert!(!plain.contains("lines_returned"));
}

#[test]
fn edit_preview_uses_raw_replacement_and_balances_both_sides() {
    let arguments = Value::object([
        ("path", Value::string("fixture.rs")),
        (
            "old_text",
            Value::string("old one\nold two\nold three\nold four\n"),
        ),
        ("new_text", Value::string("new one\nnew two\n")),
    ]);
    let result = Value::object([("bytes_written", Value::number(20))]);
    let rows = draw(
        "edit",
        &arguments,
        Some(&result),
        90,
        false,
        true,
        &Presentation::default(),
    );
    let plain = lines(&rows);
    assert!(plain[0].ends_with("✓ wrote 20 bytes"));
    assert_eq!(
        &plain[1..],
        ["│    - old one", "│    - old two", "│    + new one"]
    );
    assert!(
        !plain
            .iter()
            .any(|row| row.contains("@@") || row.contains("+2 -4"))
    );
    let painted = rows[1].paint(90);
    let (guide, content) = painted.split_once("│    ").unwrap();
    assert!(!guide.contains(&DIFF_REMOVED_SURFACE.to_string()));
    assert!(content.contains(&DIFF_REMOVED_SURFACE.to_string()));
}

#[test]
fn expanded_edit_preserves_every_source_line_and_failed_edit_shows_error() {
    let arguments = Value::object([
        ("path", Value::string("fixture.rs")),
        ("old_text", Value::string("same\nold\n")),
        ("new_text", Value::string("same\nnew\n")),
    ]);
    let result = Value::object([("bytes_written", Value::number(20))]);
    let expanded = Presentation {
        expanded: Some(true),
        ..Presentation::default()
    };
    let rows = draw(
        "edit",
        &arguments,
        Some(&result),
        90,
        true,
        false,
        &expanded,
    );
    assert_eq!(
        &lines(&rows)[1..],
        ["     - same", "     - old", "     + same", "     + new"]
    );
    assert_eq!(rows[1].origin.unwrap().line, 1);
    assert_eq!(rows[4].origin.unwrap().line, 4);
    let error = Value::object([("error", Value::string("permission denied"))]);
    let rows = draw(
        "edit",
        &arguments,
        Some(&error),
        90,
        true,
        true,
        &Presentation::default(),
    );
    let plain = lines(&rows).join("\n");
    assert!(plain.contains("permission denied"));
    assert!(!plain.contains("- old") && !plain.contains("+ new"));
}

#[test]
fn selection_and_explicit_collapse_affect_only_card_presentation() {
    let arguments = command("fixture");
    let result = Value::object([
        ("stdout", Value::string("first\nsecond\nthird\nlast")),
        ("exit_code", Value::number(0)),
    ]);
    let selected = Presentation {
        selected: true,
        expanded: Some(false),
        ..Presentation::default()
    };
    let rows = draw("bash", &arguments, Some(&result), 90, true, true, &selected);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].background, Some(TOOL_SELECTED_SURFACE));
    assert!(
        rows[0]
            .spans
            .iter()
            .any(|span| span.text.contains("fixture") && span.style == TOOL_ACCENT)
    );
    let expanded = Presentation {
        expanded: Some(true),
        ..Presentation::default()
    };
    let rows = draw(
        "bash",
        &arguments,
        Some(&result),
        90,
        true,
        false,
        &expanded,
    );
    assert_eq!(rows.len(), 5);
    assert!(lines(&rows).join("\n").contains("first"));
    assert!(!rows[0].plain().contains("preview"));
}

#[test]
fn narrow_cards_bound_lines_and_clean_controls() {
    let arguments = command("printf very-long-command");
    let result = Value::object([
        (
            "stdout",
            Value::string("\x1b[3Jlong output that exceeds the available columns"),
        ),
        ("exit_code", Value::number(0)),
    ]);
    for width in [18, 24, 40] {
        let rows = draw(
            "bash",
            &arguments,
            Some(&result),
            width,
            true,
            true,
            &Presentation::default(),
        );
        assert!(rows.iter().all(|row| text::cells(&row.plain()) <= width));
        assert!(rows.iter().all(|row| !row.plain().contains('\x1b')));
        assert!(rows.last().unwrap().plain().ends_with('…'));
    }
}
