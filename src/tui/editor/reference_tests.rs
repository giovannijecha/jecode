use super::*;

#[test]
fn references_share_attachment_chips_without_changing_the_prompt_or_editing() {
    let mut editor = Editor::default();
    editor.insert("Review $debugging @AGENTS.md ");
    let prompt = editor.prompt();
    let layout = editor.layout(80, true);
    let row = &layout.rows[0];
    assert_eq!(row.text, "Review  $debugging   @AGENTS.md  ");
    let chips: Vec<_> = row
        .labels
        .iter()
        .map(|range| &row.text[range.clone()])
        .collect();
    assert_eq!(chips, [" $debugging ", " @AGENTS.md "]);
    assert_eq!(editor.prompt(), prompt);
    assert!(editor.attachments.is_empty());

    editor.cursor = "Review $debugging @AGENTS.m".len();
    let layout = editor.layout(80, true);
    assert_eq!(layout.cursor, (0, "Review  $debugging   @AGENTS.m".len()));
    assert_eq!(layout.byte_at(0, layout.cursor.1), editor.cursor);
    editor.key(Key {
        code: 8,
        modifiers: 0,
        character: 0,
    });
    assert_eq!(editor.text, "Review $debugging @AGENTS.d ");
}

#[test]
fn reference_chips_keep_quotes_unicode_and_punctuation_when_wrapping() {
    let source = "See $review, @\"docs/my 日本 notes.md\"!";
    for width in [4, 12, 24, 80] {
        let mut editor = Editor::default();
        editor.insert(source);
        let layout = editor.layout(width, true);
        assert!(
            layout
                .rows
                .iter()
                .all(|row| text::cells(&row.text) <= width)
        );
        let chips = layout
            .rows
            .iter()
            .flat_map(|row| {
                row.labels
                    .iter()
                    .map(|range| row.text[range.clone()].to_string())
            })
            .collect::<String>();
        assert_eq!(chips, " $review  @\"docs/my 日本 notes.md\" ", "{width}");
        assert_eq!(editor.text, source);
        for (byte, _) in source.char_indices() {
            editor.cursor = byte;
            let layout = editor.layout(width, true);
            assert_eq!(
                layout.byte_at(layout.cursor.0, layout.cursor.1),
                byte,
                "{width} at {byte}"
            );
        }
    }
}

#[test]
fn editing_a_wrapped_reference_refreshes_its_whole_chip() {
    let mut editor = Editor::default();
    editor.insert("See @\"docs/my very long file.md");
    assert!(
        editor
            .layout(12, true)
            .rows
            .iter()
            .all(|row| row.labels.is_empty())
    );
    editor.insert("\"");
    let fresh = Editor {
        text: editor.text.clone(),
        cursor: editor.cursor,
        ..Editor::default()
    };
    assert_eq!(
        editor.layout(12, true).rows[0].text,
        fresh.layout(12, true).rows[0].text
    );
    let actual = editor.layout(12, true);
    let expected = fresh.layout(12, true);
    for (actual, expected) in actual.rows.iter().zip(&expected.rows) {
        assert_eq!(actual.text, expected.text);
        assert_eq!(actual.labels, expected.labels);
        assert_eq!(actual.positions, expected.positions);
    }
    assert_eq!(actual.rows.len(), expected.rows.len());
    editor.key(Key {
        code: 8,
        modifiers: 0,
        character: 0,
    });
    assert!(
        editor
            .layout(12, true)
            .rows
            .iter()
            .all(|row| row.labels.is_empty())
    );
}

#[test]
fn literal_sigils_and_slash_command_arguments_keep_their_plain_layout() {
    for source in [
        "$ @",
        r"Mail a@b.com US$5 \$review $5.00",
        "/model @literal $argument",
    ] {
        let layout = Layout::new(source, source.len(), 80, &[], true);
        assert_eq!(layout.rows[0].text, source);
        assert!(layout.rows[0].labels.is_empty());
    }
}

#[test]
fn editors_without_references_keep_sigils_as_plain_text() {
    let mut editor = Editor::default();
    editor.insert("Use $PATH and @team");
    for references in [false, true, false] {
        let layout = editor.layout(80, references);
        let row = &layout.rows[0];
        if references {
            assert_eq!(row.text, "Use  $PATH  and  @team ");
            assert_eq!(row.labels.len(), 2);
        } else {
            assert_eq!(row.text, "Use $PATH and @team");
            assert!(row.labels.is_empty());
        }
    }
}

#[test]
fn the_trailing_padding_of_a_chip_maps_to_the_reference_end() {
    let mut editor = Editor::default();
    // The caret ends under the chip's trailing padding cell.
    editor.insert("See @AGENTS.md\nabcdefghijklmno");
    let end = "See @AGENTS.md".len();
    let layout = editor.layout(80, true);
    assert_eq!(layout.rows[0].text, "See  @AGENTS.md ");
    assert_eq!(layout.byte_at(0, "See  @AGENTS.md".len()), end);
    assert!(editor.vertical(true, 80, true));
    assert_eq!(editor.cursor, end);
}

#[test]
fn cached_layout_matches_a_fresh_one_while_typing_and_deleting_references() {
    // Bare references sit on their own line, away from the quoted one.
    let source = "See @\"docs/my long file.md\" now\nand $skill-name, @src/tui/editor/layout.rs!";
    for width in [12, 20] {
        let mut editor = Editor::default();
        for character in source.chars() {
            editor.insert(&character.to_string());
            assert_cached_layout(&editor, width);
        }
        while !editor.text.is_empty() {
            editor.key(Key {
                code: 8,
                modifiers: 0,
                character: 0,
            });
            assert_cached_layout(&editor, width);
        }
    }
}

#[test]
fn cached_layout_preserves_edits_inside_and_after_quoted_references() {
    for source in [
        r#"See @"docs/my file.md" then more words"#,
        r#"See @"docs/a\"b.md" and @"docs/second file.md" end"#,
        "See @\"docs/日本 file.md\"\nthen $review, @src/main.rs!",
        r#"See @"first.md" then @"an unfinished path"#,
    ] {
        for width in [4, 12, 20] {
            for cursor in source
                .char_indices()
                .map(|(byte, _)| byte)
                .chain([source.len()])
            {
                for fragment in [
                    Some("x"),
                    Some(" "),
                    Some("\""),
                    Some("\\"),
                    Some("\n"),
                    None,
                ] {
                    let mut editor = Editor::default();
                    editor.insert(source);
                    drop(editor.layout(width, true));
                    editor.cursor = cursor;
                    if let Some(fragment) = fragment {
                        editor.insert(fragment);
                    } else {
                        editor.key(Key {
                            code: 8,
                            modifiers: 0,
                            character: 0,
                        });
                    }
                    assert_cached_layout(&editor, width);
                }
            }
        }
    }
}

fn assert_cached_layout(editor: &Editor, width: usize) {
    let actual = editor.layout(width, true);
    let expected = Layout::new(&editor.text, editor.cursor, width, &[], true);
    assert_eq!(actual.cursor, expected.cursor, "{:?}", editor.text);
    assert_eq!(actual.rows.len(), expected.rows.len(), "{:?}", editor.text);
    for (actual, expected) in actual.rows.iter().zip(&expected.rows) {
        assert_eq!(actual.text, expected.text, "{:?}", editor.text);
        assert_eq!(actual.labels, expected.labels, "{:?}", editor.text);
        assert_eq!(actual.positions, expected.positions, "{:?}", editor.text);
    }
}
