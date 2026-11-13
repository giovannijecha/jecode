//! Atomic chip layout, wrapping and incremental cache regression tests.

use super::*;

#[test]
fn references_are_single_chips_that_keep_the_prompt() {
    let editor = draft("Review $debugging @AGENTS.md ");
    let prompt = editor.prompt();
    let layout = editor.layout(80, true);
    let row = &layout.rows[0];
    assert_eq!(row.text, "Review  $debugging   @AGENTS.md  ");
    assert_eq!(shown(&editor, 80), [" $debugging ", " @AGENTS.md "]);
    // Only a chip's ends are caret positions.
    let bytes: Vec<_> = row.positions.iter().map(|(byte, _)| *byte).collect();
    for chip in syntax::chips(&editor.text) {
        assert!(bytes.contains(&chip.start) && bytes.contains(&chip.end));
        assert!(
            !bytes
                .iter()
                .any(|byte| chip.start < *byte && *byte < chip.end)
        );
    }
    assert_eq!(editor.prompt(), prompt);
    assert_eq!(prompt.text, "Review $debugging @AGENTS.md ");
    assert!(editor.attachments.is_empty());
}

#[test]
fn chips_wrap_whole_and_long_labels_shorten_on_screen_only() {
    let editor = draft("abcdefghij @AGENTS.md");
    let layout = editor.layout(16, true);
    assert_eq!(layout.rows[0].text, "abcdefghij ");
    assert_eq!(layout.rows[1].text, " @AGENTS.md ");
    let source = "See @\"docs/a very long folder/with a long file 日本.md\" end";
    for width in [4, 12, 20, 80] {
        let editor = draft(source);
        let layout = editor.layout(width, true);
        assert!(
            layout
                .rows
                .iter()
                .all(|row| text::cells(&row.text) <= width),
            "{width}"
        );
        let chips = shown(&editor, width);
        assert_eq!(chips.len(), 1, "{width}");
        if width < 50 {
            assert!(chips[0].contains('…'), "{width}: {chips:?}");
        } else {
            assert_eq!(
                chips[0],
                " @\"docs/a very long folder/with a long file 日本.md\" "
            );
        }
        assert_eq!(editor.prompt().text, source);
    }
}

#[test]
fn vertical_movement_lands_on_the_nearer_end_of_a_chip() {
    let mut editor = draft("See @AGENTS.md\nabcdefghijklmno");
    let end = "See @AGENTS.md".len();
    let layout = editor.layout(80, true);
    assert_eq!(layout.rows[0].text, "See  @AGENTS.md ");
    assert_eq!(layout.byte_at(0, "See  @AGENTS.md".len()), end);
    assert_eq!(layout.byte_at(0, "See  @A".len()), "See ".len());
    assert!(editor.vertical(true, 80, true));
    assert_eq!(editor.cursor, end);
    let mut editor = draft("See @AGENTS.md\nabcdefghijklmno");
    editor.cursor = "See @AGENTS.md\n".len() + 6;
    assert!(editor.vertical(true, 80, true));
    assert_eq!(editor.cursor, "See ".len());
}

#[test]
fn cached_layout_matches_a_fresh_one_while_typing_and_deleting_references() {
    let source = "See @\"docs/my long file.md\" now\nand $skill-name, @src/tui/editor/layout.rs!";
    for width in [12, 20] {
        let mut editor = Editor::default();
        for character in source.chars() {
            editor.compose(&character.to_string(), true);
            assert_cached_layout(&editor, width);
        }
        while !editor.text.is_empty() {
            press(&mut editor, BACKSPACE, 0);
            assert_cached_layout(&editor, width);
        }
    }
}

#[test]
fn cached_layout_follows_insertions_around_quoted_references() {
    for source in [
        r#"See @"docs/my file.md" then more words"#,
        r#"See @"docs/a\"b.md" and @"docs/second file.md" end"#,
        "See @\"docs/日本 file.md\"\nthen $review, @src/main.rs!",
        r#"See @"first.md" then @"an unfinished path"#,
    ] {
        for width in [4, 12, 20] {
            let original = draft(source);
            let chips = syntax::chips(source);
            for cursor in source
                .char_indices()
                .map(|(byte, _)| byte)
                .chain([source.len()])
                .filter(|byte| {
                    !chips
                        .iter()
                        .any(|chip| chip.start < *byte && *byte < chip.end)
                })
            {
                for fragment in ["x", " ", "\"", "\\", "\n"] {
                    for typed in [true, false] {
                        let mut editor = original.clone();
                        drop(editor.layout(width, true));
                        editor.cursor = cursor;
                        editor.compose(fragment, typed);
                        assert_cached_layout(&editor, width);
                    }
                }
            }
        }
    }
}
