use super::*;
use crate::tui::theme::INLINE_CODE;

#[test]
fn formats_messages_and_code_without_changing_the_source() {
    let source = "# Result\n**Passed** with `cargo test`.\n```rust\n    let x = 1;\n```";
    let rows = render(source, 60);
    assert_eq!(rows[0].plain(), "Result");
    assert_eq!(rows[1].plain(), "Passed with cargo test.");
    assert_eq!(rows[3].plain(), "    let x = 1;");
    assert!(source.contains("**Passed**"));
    assert!(render("\x1b[2J text", 60)[0].plain().contains("[ESC]"));
}

#[test]
fn list_items_keep_their_marker_and_hanging_indentation_when_wrapped() {
    let rows = render("- **One** two three four\n  12. five six seven eight", 14);
    let plain: Vec<_> = rows.iter().map(Line::plain).collect();
    assert_eq!(
        plain,
        [
            "• One two",
            "  three four",
            "  12. five six",
            "      seven",
            "      eight"
        ]
    );
    assert!(rows.iter().all(|row| text::cells(&row.plain()) <= 14));
    assert!(
        rows[0]
            .spans
            .iter()
            .any(|span| span.text == "One" && span.style == EMPHASIS)
    );
}

#[test]
fn code_spans_handle_literal_ticks_bold_nesting_escapes_and_unmatched_markers() {
    let source = "Use ``a`b`` and **`let x`**; \\*literal\\*; `unclosed **";
    let row = &render(source, 120)[0];
    assert_eq!(row.plain(), "Use a`b and let x; *literal*; `unclosed **");
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "a`b" && span.style == INLINE_CODE)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "let x" && span.style == INLINE_CODE)
    );
    assert_eq!(
        render("**`a ** b` remains bold**", 80)[0].plain(),
        "a ** b remains bold"
    );
    assert_eq!(
        render(r"C:\Users\fixture", 80)[0].plain(),
        r"C:\Users\fixture"
    );
    assert_eq!(render("literal ****", 80)[0].plain(), "literal ****");
    assert_eq!(render("literal ** **", 80)[0].plain(), "literal ** **");
}

#[test]
fn fences_require_matching_markers_and_never_format_the_code_as_markdown() {
    let source = "````unknown\n```\n**literal**\n```` suffix\n````\n~~~json\n{\"n\": 40}\n~~~";
    let rows = render(source, 80);
    let plain: Vec<_> = rows.iter().map(Line::plain).collect();
    assert_eq!(
        plain,
        [
            "unknown",
            "```",
            "**literal**",
            "```` suffix",
            "",
            "json",
            "{\"n\": 40}",
            ""
        ]
    );
    assert!(
        rows[2]
            .spans
            .iter()
            .all(|span| span.style == crate::tui::theme::CODE_TEXT)
    );
    assert_eq!(
        render("```rust\nlet x = 1;", 80).last().unwrap().plain(),
        "let x = 1;"
    );
}

#[test]
fn quotes_and_narrow_code_preserve_visible_content_and_widths() {
    let source = "> **A** longer quotation\n```rust\n    let città = \"🙂\";\n```";
    for columns in [6, 14, 30] {
        let rows = render(source, columns);
        assert!(rows.iter().all(|row| text::cells(&row.plain()) <= columns));
        assert!(rows[0].plain().starts_with("│ A"));
        let code: String = rows
            .iter()
            .skip_while(|row| row.plain() != "rust")
            .skip(1)
            .take_while(|row| !row.plain().is_empty())
            .map(Line::plain)
            .collect();
        assert_eq!(code, "    let città = \"🙂\";");
    }
}

#[test]
fn headings_tasks_and_inline_formatting_keep_their_visible_text() {
    let source = "## Title ##\nSubtitle\n===\n- [ ] **pending**\n- [x] _done_\n>~~old~~ and [docs](https://example.test/a)";
    let rows = render(source, 100);
    assert_eq!(
        rows.iter().map(Line::plain).collect::<Vec<_>>(),
        [
            "Title",
            "Subtitle",
            "☐ pending",
            "☑ done",
            "│ old and docs (https://example.test/a)"
        ]
    );
    assert_eq!(rows[1].spans[0].style, EMPHASIS);
    assert_eq!(render("## C#\nLiteral snake_case", 60)[0].plain(), "C#");
    assert!(source.contains("~~old~~"));
}

#[test]
fn partial_table_delimiters_remain_visible_until_a_valid_table_exists() {
    for delimiter in ["|", "| --- | unfinished |", "| --- | --- | --- |"] {
        let source = format!("| A | B |\n{delimiter}");
        let rows = render(&source, 80);
        assert_eq!(rows[0].plain(), "| A | B |");
        assert_eq!(rows[1].plain(), delimiter);
    }
    assert_eq!(
        render("| A | B |\n|---|---|", 80)
            .iter()
            .map(Line::plain)
            .collect::<Vec<_>>(),
        ["A   B", "──  ──"]
    );
}
