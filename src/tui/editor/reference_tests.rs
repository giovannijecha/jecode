use super::*;

#[test]
fn references_share_attachment_chips_without_changing_the_prompt_or_editing() {
    let mut editor = Editor::default();
    editor.insert("Review $debugging @AGENTS.md ");
    let prompt = editor.prompt();
    let layout = editor.layout(80);
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
    let layout = editor.layout(80);
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
        let layout = editor.layout(width);
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
            let layout = editor.layout(width);
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
            .layout(12)
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
        editor.layout(12).rows[0].text,
        fresh.layout(12).rows[0].text
    );
    let actual = editor.layout(12);
    let expected = fresh.layout(12);
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
            .layout(12)
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
        let layout = Layout::new(source, source.len(), 80, &[]);
        assert_eq!(layout.rows[0].text, source);
        assert!(layout.rows[0].labels.is_empty());
    }
}
