use super::*;
use crate::tui::theme::{ACCENT, BODY, CURSOR};

#[test]
fn the_block_caret_keeps_its_background_inside_a_full_width_panel() {
    use crate::tui::theme::USER_BACKGROUND;
    let line = Line::new("città", BODY)
        .on(USER_BACKGROUND)
        .caret(2, CURSOR);
    let painted = line.paint(12);
    assert!(painted.contains(&format!("\x1b[{CURSOR}mt")));
    assert!(painted.contains(&format!("\x1b[{USER_BACKGROUND}mci")));
    assert!(painted.contains(&format!("\x1b[{USER_BACKGROUND}m{}", " ".repeat(7))));
    assert_eq!(visible(&painted).trim_end(), "città");
}

#[test]
fn the_block_caret_preserves_unicode_glyphs_and_does_not_change_the_source() {
    let mut line = Line::new("› ", ACCENT);
    line.push("ae\u{301}界z", BODY);
    let source = line.plain();
    let accented = line.caret(3, CURSOR);
    assert_eq!(accented.plain(), source);
    assert_eq!(
        accented
            .spans
            .iter()
            .find(|span| span.style == CURSOR)
            .unwrap()
            .text,
        "e\u{301}"
    );
    let wide = line.caret(5, CURSOR);
    assert_eq!(wide.plain(), source);
    assert_eq!(
        wide.spans
            .iter()
            .find(|span| span.style == CURSOR)
            .unwrap()
            .text,
        "界"
    );
    let end = line.caret(text::cells(&source), CURSOR);
    assert_eq!(end.plain(), format!("{source} "));
    assert_eq!(end.spans.last().unwrap().style, CURSOR);
    assert_eq!(line.plain(), source);
    assert_eq!(Line::default().caret(0, CURSOR).plain(), " ");
}

#[test]
fn wraps_words_with_their_styles_and_preserves_code_indentation() {
    let mut line = Line::new("a readable ", BODY);
    line.push("sentence", ACCENT);
    let rows = line.wrap(12, true);
    assert_eq!(
        rows.iter().map(Line::plain).collect::<Vec<_>>(),
        ["a readable", "sentence"]
    );
    assert_eq!(rows[1].spans[0].style, ACCENT);
    assert_eq!(
        Line::new("five six seven", BODY)
            .wrap(8, true)
            .iter()
            .map(Line::plain)
            .collect::<Vec<_>>(),
        ["five six", "seven"]
    );
    assert_eq!(
        Line::new("    abcdef", BODY).wrap(6, false)[0].plain(),
        "    ab"
    );
}

#[test]
fn prose_word_wrap_preserves_inline_code_spaces() {
    let code = Line::new("a   b", INLINE_CODE);
    let rows = code.wrap(2, true);
    assert_eq!(
        rows.iter().map(Line::plain).collect::<String>(),
        code.plain()
    );
    assert!(rows.iter().all(|row| text::cells(&row.plain()) <= 2));
    assert!(rows.iter().all(|row| row.spans[0].style == INLINE_CODE));

    let mut mixed = Line::new("prose ", BODY);
    mixed.push("a b", INLINE_CODE);
    assert_eq!(
        mixed
            .clone()
            .into_wrapped(2, true)
            .map(|row| row.plain())
            .collect::<Vec<_>>(),
        mixed
            .wrap(2, true)
            .iter()
            .map(Line::plain)
            .collect::<Vec<_>>()
    );
    assert!(
        mixed
            .wrap(2, true)
            .iter()
            .any(|row| row.plain().contains(' '))
    );
}

#[test]
fn wrapped_iterator_emits_rows_incrementally_and_retains_trailing_newlines() {
    let mut wrapped = Line::new("a\n", BODY).into_wrapped(2, true);
    assert_eq!(wrapped.next().unwrap().plain(), "a");
    assert_eq!(wrapped.next().unwrap().plain(), "");
    assert!(wrapped.next().is_none());
    assert!(wrapped.next().is_none());
}

#[test]
fn joined_emoji_remains_whole_across_styles_wraps_and_caret() {
    let mut line = Line::new("A\u{1f469}", BODY);
    line.push("\u{200d}\u{1f4bb}B", ACCENT);
    let original = line.plain();
    assert_eq!(text::cells(&original), 4);
    let rows = line.wrap(2, false);
    assert_eq!(
        rows.iter().map(Line::plain).collect::<Vec<_>>(),
        ["A", "\u{1f469}\u{200d}\u{1f4bb}", "B"]
    );
    assert_eq!(rows[1].spans[0].style, BODY);
    assert_eq!(rows[1].spans[1].style, ACCENT);
    assert_eq!(line.shortened(3).plain(), "A…");
    let caret = line.caret(2, CURSOR);
    assert_eq!(caret.plain(), original);
    assert_eq!(
        caret
            .spans
            .iter()
            .find(|span| span.style == CURSOR)
            .unwrap()
            .text,
        "\u{1f469}\u{200d}\u{1f4bb}"
    );
    assert_eq!(visible(&line.paint(2)), "A");
    assert_eq!(visible(&line.paint(3)), "A\u{1f469}\u{200d}\u{1f4bb}");
    assert_eq!(line.plain(), original);
}

fn visible(painted: &str) -> String {
    let mut result = String::new();
    let mut escaped = false;
    for character in painted.chars() {
        if character == '\u{1b}' {
            escaped = true;
        } else if escaped {
            if character == 'm' {
                escaped = false;
            }
        } else {
            result.push(character);
        }
    }
    result
}
