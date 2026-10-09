use super::*;

fn render(
    name: &str,
    arguments: &Value,
    summary: &str,
    result: Option<&Value>,
    last: bool,
    columns: usize,
) -> Vec<Line> {
    super::render(
        Tool {
            name,
            arguments,
            summary,
            result,
            last,
            preview: true,
            presentation: &Presentation::default(),
        },
        columns,
    )
}

fn output() -> Value {
    Value::object([
        (
            "stdout",
            Value::string("first\nsecond\nthird\nfourth\nlast"),
        ),
        ("stderr", Value::string("important error\nmore error")),
        ("exit_code", Value::number(7)),
        ("stdout_bytes", Value::number(70000)),
        ("stdout_truncated", Value::Bool(true)),
        ("output_limit_bytes", Value::number(65536)),
    ])
}

#[test]
fn previews_errors_and_separates_hidden_lines_from_discarded_capture() {
    let value = output();
    let arguments = Value::object([("command", Value::string("fixture-command"))]);
    let rows = render(
        "bash",
        &arguments,
        "exit 7; output truncated",
        Some(&value),
        true,
        90,
    );
    let text = rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n");
    assert!(text.contains("important error"));
    assert!(rows[0].plain().contains("preview 3 / 7"));
    assert!(!text.contains("omitted"));
    assert!(text.contains("capture truncated: 65536 of 70000"));
    assert!(text.contains("last"));
    assert!(rows[0].plain().contains("fixture-command"));
    assert_eq!(
        value.get("stdout").and_then(Value::as_str),
        Some("first\nsecond\nthird\nfourth\nlast")
    );
}

#[test]
fn reads_show_the_returned_range_and_next_offset_without_exposing_a_field_dump() {
    let arguments = Value::object([
        ("path", Value::string("fixture.rs")),
        ("offset", Value::number(4)),
    ]);
    let result = Value::object([
        ("content", Value::string("4: one\n5: two\n")),
        ("lines_returned", Value::number(2)),
        ("next_offset", Value::number(6)),
    ]);
    let rows = render("read", &arguments, "complete", Some(&result), true, 90);
    let text = rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n");
    assert!(text.contains("lines 4–5"));
    assert!(text.contains("5: two"));
    assert!(text.contains("from line 6"));
    assert!(!text.contains("lines_returned"));
}

#[test]
fn byte_pages_show_their_real_range_without_inventing_global_line_numbers() {
    let arguments = Value::object([
        ("path", Value::string("output:1-2-3:stdout")),
        ("byte_offset", Value::number(5000)),
    ]);
    let result = Value::object([
        ("content", Value::string("middle\nmore")),
        ("offset", Value::Null),
        ("byte_offset", Value::number(5000)),
        ("bytes_returned", Value::number(11)),
        ("lines_returned", Value::number(2)),
        ("next_byte_offset", Value::number(5011)),
    ]);
    let rows = render("read", &arguments, "complete", Some(&result), true, 100);
    let text = rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n");
    assert!(text.contains("bytes 5000–5010"));
    assert!(text.contains("from byte 5011"));
    assert!(!text.contains("lines 1–2"));
}

#[test]
fn failed_writes_show_the_error_instead_of_an_apparently_written_preview() {
    let arguments = Value::object([
        ("path", Value::string("fixture.txt")),
        ("content", Value::string("never written")),
    ]);
    let result = Value::object([("error", Value::string("read-only file"))]);
    let rows = render(
        "write",
        &arguments,
        "error: read-only file",
        Some(&result),
        true,
        80,
    );
    let text = rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n");
    assert!(text.contains("read-only file"));
    assert!(!text.contains("never written"));
}

#[test]
fn textual_outcomes_distinguish_execution_without_a_tool_spinner() {
    let arguments = Value::object([("command", Value::string("fixture-command"))]);
    let cases = [
        (None, "running", TOOL_ACCENT),
        (
            Some(Value::object([("exit_code", Value::number(0))])),
            "exit 0",
            GOOD,
        ),
        (
            Some(Value::object([("exit_code", Value::number(7))])),
            "exit 7",
            BAD,
        ),
        (
            Some(Value::object([("timed_out", Value::Bool(true))])),
            "timed out",
            BAD,
        ),
        (
            Some(Value::object([("cancelled", Value::Bool(true))])),
            "cancelled",
            WARNING,
        ),
        (
            Some(Value::object([("error", Value::string("fixture failure"))])),
            "error",
            BAD,
        ),
    ];
    for (result, status, style) in cases {
        let rows = render("bash", &arguments, status, result.as_ref(), true, 90);
        let title = &rows[0];
        let plain = title.plain();
        assert!(plain.starts_with("└─ bash  fixture-command"));
        assert!(!plain.contains(['▸', '▾', '●', '•']));
        assert_eq!(
            title
                .spans
                .iter()
                .find(|span| span
                    .text
                    .contains(if status == "error" { "failed" } else { status }))
                .unwrap()
                .style,
            style
        );
        assert!(plain.ends_with(if status == "error" { "failed" } else { status }));
    }
}

#[test]
fn command_previews_show_the_retained_tail_and_keep_capture_notices_separate() {
    let arguments = Value::object([("command", Value::string("cargo test"))]);
    let value = Value::object([
        (
            "stdout",
            Value::string("earlier output\nrunning tests\none test passed\ntest result: ok"),
        ),
        ("exit_code", Value::number(0)),
    ]);
    let rows = render("bash", &arguments, "exit 0", Some(&value), false, 90);
    let plain = rows.iter().map(Line::plain).collect::<Vec<_>>();
    assert!(plain[0].starts_with("├─ bash  cargo test"));
    assert!(plain[0].ends_with("✓ exit 0 · preview 3 / 4"));
    assert_eq!(plain[1], "│    running tests");
    assert_eq!(rows.len(), 4);
    assert_eq!(plain.last().unwrap(), "│    test result: ok");
    assert!(!plain.iter().any(|row| row.contains("earlier output")));
    assert_eq!(rows[0].spans[0].style, TOOL_GUIDE);
    assert_eq!(rows[1].spans[0].style, TOOL_GUIDE);
    assert!(
        value
            .get("stdout")
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("earlier output")
    );
}

#[test]
fn successful_commands_keep_stderr_visible_beside_the_stdout_tail() {
    let arguments = Value::object([("command", Value::string("fixture"))]);
    let result = Value::object([
        ("stdout", Value::string("earlier\nfirst\nsecond\nlast")),
        ("stderr", Value::string("fixture warning")),
        ("exit_code", Value::number(0)),
    ]);
    let rows = render("bash", &arguments, "exit 0", Some(&result), true, 90);
    let plain = rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n");
    assert!(plain.contains("second"));
    assert!(plain.contains("last"));
    assert!(plain.contains("stderr: fixture warning"));
    assert!(rows[0].plain().contains("preview 3 / 5"));
    assert_eq!(rows.len(), 4);
}

#[test]
fn read_previews_keep_the_start_of_the_returned_range() {
    let arguments = Value::object([("path", Value::string("fixture.rs"))]);
    let result = Value::object([
        (
            "content",
            Value::string("1: first\n2: second\n3: third\n4: fourth"),
        ),
        ("lines_returned", Value::number(4)),
    ]);
    let rows = render("read", &arguments, "complete", Some(&result), true, 90);
    let plain = rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n");
    assert!(plain.contains("1: first"));
    assert!(!plain.contains("4: fourth"));
    assert!(rows[0].plain().contains("preview 3 / 4"));
    assert_eq!(rows.len(), 4);
}

#[test]
fn edit_previews_balance_removed_and_added_content_and_keep_guides_unshaded() {
    let arguments = Value::object([
        ("path", Value::string("fixture.rs")),
        (
            "old_text",
            Value::string("old one\nold two\nold three\nold four\n"),
        ),
        ("new_text", Value::string("new one\nnew two\n")),
    ]);
    let result = Value::object([("bytes_written", Value::number(20))]);
    let rows = render(
        "edit",
        &arguments,
        "wrote 20 bytes",
        Some(&result),
        false,
        90,
    );
    let plain = rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n");
    assert!(plain.contains("- old one"));
    assert!(plain.contains("+ new one"));
    assert!(rows[0].plain().contains("preview 3 / 6"));
    assert_eq!(rows.len(), 4);
    assert!(!plain.contains("@@"));
    let removal = rows
        .iter()
        .find(|row| row.plain().contains("- old one"))
        .unwrap();
    let painted = removal.paint(89);
    let (guide, content) = painted.split_once("│    ").unwrap();
    assert!(!guide.contains(crate::tui::theme::DIFF_REMOVED_BACKGROUND));
    assert!(content.contains(crate::tui::theme::DIFF_REMOVED_BACKGROUND));
}

#[test]
fn deletion_previews_do_not_invent_an_added_line() {
    let arguments = Value::object([
        ("path", Value::string("fixture.rs")),
        ("old_text", Value::string("removed content\n")),
        ("new_text", Value::string("")),
    ]);
    let result = Value::object([("bytes_written", Value::number(0))]);
    let rows = render("edit", &arguments, "wrote 0 bytes", Some(&result), true, 90);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].plain(), "     - removed content");
}

#[test]
fn narrow_cards_wrap_commands_and_shorten_previews_without_leaking_controls() {
    let arguments = Value::object([("command", Value::string("printf very-long-command"))]);
    let result = Value::object([
        (
            "stdout",
            Value::string("\x1b[3Jlong output that exceeds the available columns"),
        ),
        ("exit_code", Value::number(0)),
    ]);
    for last in [false, true] {
        for width in [18, 24, 40] {
            let rows = render("bash", &arguments, "exit 0", Some(&result), last, width);
            assert!(rows.len() >= 3);
            assert!(rows.last().unwrap().plain().ends_with('…'));
            assert!(
                rows[0]
                    .plain()
                    .starts_with(if last { "└─ bash" } else { "├─ bash" })
            );
            assert!(rows.iter().all(|row| text::cells(&row.plain()) <= width));
            assert!(rows.iter().all(|row| !row.plain().contains('\x1b')));
            assert!(!rows.iter().any(|row| row.plain().contains("Ctrl+O")));
        }
    }
}

#[test]
fn expanding_a_tool_removes_preview_metadata_and_shows_all_retained_content() {
    let arguments = Value::object([("command", Value::string("fixture"))]);
    let result = Value::object([
        ("stdout", Value::string("first\nsecond\nthird\nlast")),
        ("exit_code", Value::number(0)),
    ]);
    let mut presentation = Presentation::default();
    for expanded in [None, Some(true), Some(false)] {
        presentation.expanded = expanded;
        let rows = super::render(
            Tool {
                name: "bash",
                arguments: &arguments,
                summary: "exit 0",
                result: Some(&result),
                last: true,
                preview: true,
                presentation: &presentation,
            },
            80,
        );
        let plain = rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n");
        assert_eq!(
            rows[0].plain().contains("preview 3 / 4"),
            expanded.is_none()
        );
        assert_eq!(plain.contains("first"), expanded == Some(true));
        assert_eq!(
            rows.len(),
            match expanded {
                None => 4,
                Some(true) => 5,
                Some(false) => 1,
            }
        );
        assert!(!plain.contains("omitted"));
    }
    assert_eq!(
        result.get("stdout").and_then(Value::as_str),
        Some("first\nsecond\nthird\nlast")
    );
}
