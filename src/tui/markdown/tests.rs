use super::*;
use crate::tui::theme::{INLINE_CODE, INLINE_CODE_SURFACE};

#[test]
fn streamed_appends_match_full_markdown_after_each_ambiguous_boundary() {
    let chunks = [
        "Paragraph",
        "\n=",
        "==\n| A | B |",
        "\n|---|---|",
        "\n| short | x |",
        "\n| a much wider value | y |",
        "\n\n```rust\n/*",
        " open comment\ncontinued",
        " */\nlet value = 3;\n```",
        "\n- [x] done",
        "\n\nsoft",
        " wrap\nacross *two",
        "\nlines*  ",
        "\nafter a break\n> quoted",
        "\n> more",
        "\n",
    ];
    let mut source = String::new();
    let mut stream = Stream::new("", 28);
    let mut incremental = Vec::new();
    for chunk in chunks {
        source.push_str(chunk);
        if let Some(keep) = stream.update(&source, 28) {
            incremental.truncate(keep);
        }
        for _ in 0..100 {
            stream.advance(&mut incremental, 4);
            if stream.ready() {
                break;
            }
        }
        assert!(stream.ready(), "stream stalled after {chunk:?}");
        assert_eq!(incremental, render(&source, 28), "after {chunk:?}");
    }
}

#[test]
fn streamed_replacement_and_trailing_whitespace_match_full_render() {
    let mut stream = Stream::new("first  ", 16);
    let mut lines = Vec::new();
    stream.advance(&mut lines, 8);
    for source in [
        "first  more",
        "first  more\r",
        "first  more\r\nnext",
        "other\ntext",
    ] {
        if let Some(keep) = stream.update(source, 16) {
            lines.truncate(keep);
        }
        while !stream.ready() {
            stream.advance(&mut lines, 8);
        }
        assert_eq!(lines, render(source, 16));
    }
}

#[test]
fn formats_messages_and_code_without_changing_the_source() {
    let source = "# Result\n**Passed** with `cargo test`.\n```rust\n    let x = 1;\n```";
    let rows = render(source, 60);
    assert_eq!(rows[0].plain(), "Result");
    assert_eq!(rows[1].plain(), "");
    assert_eq!(rows[2].plain(), "Passed with cargo test.");
    assert_eq!(rows[4].plain(), "     let x = 1;");
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
            .any(|span| span.text == "One" && span.style == EMPHASIS.bold())
    );
}

#[test]
fn code_spans_handle_literal_ticks_bold_nesting_escapes_and_unmatched_markers() {
    let source = "Use ``a`b`` and **`let x`**; \\*literal\\*; `unclosed **";
    let row = &render(source, 120)[0];
    assert_eq!(row.plain(), "Use a`b and let x; *literal*; `unclosed **");
    assert!(row.spans.iter().any(|span| span.text == "a`b"
        && span.style == INLINE_CODE
        && span.background == Some(INLINE_CODE_SURFACE)));
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
            " unknown",
            " ```",
            " **literal**",
            " ```` suffix",
            "",
            " json",
            " {\"n\": 40}",
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
        " let x = 1;"
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
            .filter(|row| row.background == Some(CODE_SURFACE))
            .skip(1)
            .take_while(|row| !row.plain().is_empty())
            .map(|row| row.plain().strip_prefix(' ').unwrap().to_owned())
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
            "",
            "Subtitle",
            "",
            "☐ pending",
            "☑ done",
            "│ old and docs (https://example.test/a)"
        ]
    );
    // Levels: H1 accent, H2 emphasis, H3 and below body, all bold.
    assert_eq!(rows[0].spans[0].style, EMPHASIS.bold());
    assert_eq!(rows[2].spans[0].style, ACCENT.bold());
    assert_eq!(render("### Small", 60)[0].spans[0].style, BODY.bold());
    assert_eq!(render("Top\n--", 60)[0].spans[0].style, EMPHASIS.bold());
    assert_eq!(render("## Last", 60).len(), 1);
    assert_eq!(render("## C#\nLiteral snake_case", 60)[0].plain(), "C#");
    assert!(source.contains("~~old~~"));
}

#[test]
fn partial_table_delimiters_remain_visible_until_a_valid_table_exists() {
    for delimiter in ["|", "| --- | unfinished |", "| --- | --- | --- |"] {
        let source = format!("| A | B |\n{delimiter}");
        let rows = render(&source, 80);
        assert_eq!(rows[0].plain(), format!("| A | B | {delimiter}"));
    }
    assert_eq!(
        render("| A | B |\n|---|---|", 80)
            .iter()
            .map(Line::plain)
            .collect::<Vec<_>>(),
        ["A   B", "──  ──"]
    );
}

#[test]
fn soft_breaks_join_paragraphs_and_hard_breaks_keep_rows() {
    let source = "One *two\nthree* four\nfive  \nsix\\\nseven\n- item\n  more\n# Next";
    let rows = render(source, 80);
    let plain: Vec<_> = rows.iter().map(Line::plain).collect();
    assert_eq!(
        plain,
        [
            "One two three four five  ",
            "six\\",
            "seven",
            "• item more",
            "Next"
        ]
    );
    assert_eq!(rows[0].origin.unwrap().line, 0);
    assert_eq!(rows[1].origin.unwrap().line, source.find("six").unwrap());
    assert!(rows[0].spans.iter().any(|span| span.text == "two three"));
    let plain = |source, columns| {
        render(source, columns)
            .iter()
            .map(Line::plain)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        plain("first line\nsecond line", 16),
        ["first line", "second line"]
    );
    assert_eq!(
        plain("first line\nsecond line", 24),
        ["first line second line"]
    );
    assert_eq!(plain("A\n=\nB\ntitle\n---", 80), ["A", "", "B", "title"]);
    assert_eq!(
        plain("text\n| A | B |\n|---|---|", 80),
        ["text", "A   B", "──  ──"]
    );
}

#[test]
fn quotes_repeat_a_muted_bar_and_join_continuation_lines() {
    let rows = render("> one\n> two\nafter", 80);
    let plain: Vec<_> = rows.iter().map(Line::plain).collect();
    assert_eq!(plain, ["│ one two after"]);
    let paragraphs: Vec<_> = render("> one\n>\n> - two\n\nafter", 80)
        .iter()
        .map(Line::plain)
        .collect();
    assert_eq!(paragraphs, ["│ one", "│ ", "│ - two", "", "after"]);
    assert_eq!(rows[0].spans[0].style, MUTED);
    assert!(rows[0].spans.iter().all(|span| span.style == MUTED));
    let wrapped: Vec<_> = render("> alpha beta gamma", 10)
        .iter()
        .map(Line::plain)
        .collect();
    assert_eq!(wrapped, ["│ alpha", "│ beta", "│ gamma"]);
}

#[test]
fn code_panels_pad_one_cell_on_the_code_surface() {
    let rows = render("```text\nabcdefghij\n```", 10);
    let plain: Vec<_> = rows.iter().map(Line::plain).collect();
    assert_eq!(plain, [" text", " abcdefgh", " ij", ""]);
    assert!(rows.iter().all(|row| row.background == Some(CODE_SURFACE)));
}

#[test]
fn narrow_code_panels_keep_every_glyph_visible() {
    for (columns, expected) in [
        (2, vec!["ab", "界", "c"]),
        (3, vec!["ab", "界c"]),
        (4, vec![" ab", " 界", " c"]),
    ] {
        let rows = render("```\nab界c\n```", columns);
        let content: Vec<_> = rows[1..rows.len() - 1].iter().map(Line::plain).collect();
        assert_eq!(content, expected, "width {columns}");
        assert!(rows.iter().all(|row| text::cells(&row.plain()) <= columns));
        assert!(rows.iter().all(|row| row.background == Some(CODE_SURFACE)));
    }
}

#[test]
fn empty_prose_and_blank_code_lines_match_source_rows() {
    let empty = render("", 40);
    assert_eq!(empty.len(), 1);
    assert_eq!(empty[0].plain(), "");
    assert_eq!(empty[0].origin.unwrap().line, 0);

    let code = render("```rust\n\n```", 40);
    assert_eq!(
        code.iter().map(Line::plain).collect::<Vec<_>>(),
        [" rust", "", ""]
    );
    assert!(code.iter().all(|row| row.background == Some(CODE_SURFACE)));
}
