use super::*;

#[test]
fn code_cards_wrap_at_tokens_under_the_indentation_without_a_row_cap() {
    let source = "```js\n    const targets = document.querySelectorAll('.intro > p, .stats > div, .contact-form, footer');\n```";
    let rows: Vec<_> = render(source, 40).iter().map(Line::plain).collect();
    assert_eq!(
        rows,
        [
            " js",
            "     const targets =",
            "       document.querySelectorAll(",
            "       '.intro > p, .stats > div,",
            "       .contact-form, footer');",
            ""
        ]
    );
    // Markup breaks before a tag rather than after `<b>`.
    let rows: Vec<_> = render(
        "```html\n<li>Item number one<b>bold words</b></li>\n```",
        26,
    )
    .iter()
    .map(Line::plain)
    .collect();
    assert_eq!(
        rows[1..3],
        [" <li>Item number one", "   <b>bold words</b></li>"]
    );
}

#[test]
fn code_panels_pad_one_cell_on_the_code_surface() {
    let rows = render("```text\nabcdefghij\n```", 10);
    let plain: Vec<_> = rows.iter().map(Line::plain).collect();
    assert_eq!(plain, [" text", " abcdefgh", "   ij", ""]);
    assert!(rows.iter().all(|row| row.background == Some(CODE_SURFACE)));
}

#[test]
fn narrow_code_panels_keep_every_glyph_visible() {
    for (columns, expected) in [
        (2, vec!["ab", "界", " c"]),
        (3, vec!["ab", " 界", " c"]),
        (4, vec![" ab", " 界", "  c"]),
    ] {
        let rows = render("```\nab界c\n```", columns);
        let content: Vec<_> = rows[1..rows.len() - 1].iter().map(Line::plain).collect();
        assert_eq!(content, expected, "width {columns}");
        assert!(rows.iter().all(|row| text::cells(&row.plain()) <= columns));
        assert!(rows.iter().all(|row| row.background == Some(CODE_SURFACE)));
    }
}
