use super::*;
fn key(code: u16, modifiers: u8) -> Key {
    Key {
        code,
        modifiers,
        character: 0,
    }
}

#[test]
fn unicode_edits_and_word_controls_preserve_character_boundaries() {
    let mut editor = Editor::default();
    editor.insert("città\r\n🙂 word");
    editor.key(key(8, 4));
    assert_eq!(editor.text, "città\n🙂 ");
    editor.key(key(37, 4));
    assert_eq!(editor.cursor, "città\n".len());
    editor.key(key(46, 0));
    assert_eq!(editor.text, "città\n ");
    editor.key(key(65, 4));
    editor.key(key(74, 4));
    assert_eq!(editor.text, "città\n\n ");
    editor.key(key(36, 4));
    editor.insert("è ");
    assert_eq!(editor.take(), "è città\n\n ");
}

#[test]
fn visual_movement_uses_cells_and_remembers_the_column_on_short_lines() {
    let mut editor = Editor::default();
    editor.insert("abcdef\n界x\nabcdef");
    assert!(editor.vertical(true, 8));
    assert!(editor.vertical(true, 8));
    assert_eq!(&editor.text[editor.cursor..], "\n界x\nabcdef");
    assert!(editor.vertical(false, 8));
    assert!(editor.vertical(false, 8));
    assert_eq!(editor.cursor, editor.text.len());
    let layout = Layout::new("abcd界z", 4, 4);
    assert_eq!(layout.cursor, (1, 0));
    assert_eq!(layout.rows[1].text, "界z");
}

#[test]
fn oversized_paste_is_rejected_and_recovery_never_truncates() {
    let mut editor = Editor::default();
    editor.insert("original");
    assert!(!editor.insert(&"x".repeat(LIMIT)));
    assert_eq!(editor.text, "original");
    editor.replace("x".repeat(LIMIT + 1));
    assert_eq!(editor.text.len(), LIMIT + 1);
    assert!(!editor.insert("y"));
    editor.key(key(8, 0));
    editor.key(key(8, 0));
    assert!(editor.insert("y"));
}

#[test]
fn search_viewport_keeps_the_insertion_cell_visible_with_wide_characters() {
    let mut editor = Editor::default();
    editor.insert("abc界界z");
    let (visible, cursor) = editor.viewport(5, false);
    assert_eq!(visible, "界z");
    assert_eq!(cursor, 3);
    editor.cursor = "abc界".len();
    let (visible, cursor) = editor.viewport(5, false);
    assert_eq!(visible, "bc界");
    assert_eq!(cursor, 4);
    let (masked, cursor) = editor.viewport(4, true);
    assert_eq!(masked, "****");
    assert_eq!(cursor, 3);
}

#[test]
fn joined_emoji_uses_one_layout_position_and_edits_as_a_whole_glyph() {
    let joined = "\u{1f469}\u{1f3fd}\u{200d}\u{1f4bb}";
    let source = format!("A{joined}B");
    let mut editor = Editor::default();
    editor.insert(&source);
    let layout = editor.layout(4);
    assert_eq!(layout.rows[0].text, source);
    assert_eq!(layout.rows[0].positions.len(), 4);
    assert_eq!(layout.byte_at(0, 2), 1);
    assert_eq!(layout.byte_at(0, 3), 1 + joined.len());
    editor.key(key(37, 0));
    editor.key(key(8, 0));
    assert_eq!(editor.text, "AB");
    assert_eq!(editor.cursor, 1);
    assert_eq!(editor.viewport(4, false), ("AB".into(), 1));
}

#[test]
fn local_navigation_keeps_sequences_whole_near_the_end_of_a_large_draft() {
    let mut editor = Editor::default();
    editor.insert(&"a".repeat(LIMIT - 128));
    let prefix = editor.cursor;
    editor.insert("e\u{301}\u{1f1ee}\u{1f1f9}\u{1f469}\u{1f3fd}\u{200d}\u{1f4bb}z");
    editor.key(key(37, 0));
    assert_eq!(editor.text[editor.cursor..].chars().next(), Some('z'));
    editor.key(key(37, 0));
    assert_eq!(
        &editor.text[editor.cursor..],
        "\u{1f469}\u{1f3fd}\u{200d}\u{1f4bb}z"
    );
    editor.key(key(37, 0));
    assert_eq!(
        &editor.text[editor.cursor..],
        "\u{1f1ee}\u{1f1f9}\u{1f469}\u{1f3fd}\u{200d}\u{1f4bb}z"
    );
    editor.key(key(37, 0));
    assert_eq!(editor.cursor, prefix);
    editor.key(key(39, 0));
    assert_eq!(
        &editor.text[editor.cursor..],
        "\u{1f1ee}\u{1f1f9}\u{1f469}\u{1f3fd}\u{200d}\u{1f4bb}z"
    );

    editor.cursor = prefix;
    for _ in 0..2048 {
        editor.key(key(37, 0));
    }
    assert_eq!(editor.cursor, prefix - 2048);
    for _ in 0..2048 {
        editor.key(key(39, 0));
    }
    assert_eq!(editor.cursor, prefix);
}
