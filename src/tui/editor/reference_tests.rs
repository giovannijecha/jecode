//! Reference chips in the chat composer: single elements for the caret, deletion,
//! insertion and layout. Other editors keep ordinary character editing.

use super::*;
use crate::tui::references::syntax;

mod attachment_tests;
mod layout_tests;

const LEFT: u16 = 37;
const RIGHT: u16 = 39;
const UP: u16 = 38;
const DOWN: u16 = 40;
const BACKSPACE: u16 = 8;
const DELETE: u16 = 46;
const HOME: u16 = 36;
const END: u16 = 35;
const CTRL: u8 = 4;

fn key(code: u16, modifiers: u8) -> Key {
    Key {
        code,
        modifiers,
        character: 0,
    }
}

/// A draft pasted at once: its references are chips straight away.
fn draft(text: &str) -> Editor {
    let mut editor = Editor::default();
    assert!(editor.compose(text, false));
    editor
}

fn typed(editor: &mut Editor, text: &str) {
    for character in text.chars() {
        assert!(editor.compose(&character.to_string(), true));
    }
}

fn press(editor: &mut Editor, code: u16, modifiers: u8) {
    assert!(editor.compose_key(key(code, modifiers)));
}

/// The chip texts on screen, in order.
fn shown(editor: &Editor, width: usize) -> Vec<String> {
    editor
        .layout(width, true)
        .rows
        .iter()
        .flat_map(|row| {
            row.labels
                .iter()
                .map(|range| row.text[range.clone()].to_string())
        })
        .collect()
}

fn references(text: &str) -> Vec<String> {
    syntax::chips(text)
        .into_iter()
        .map(|chip| text[chip].to_string())
        .collect()
}

fn image() -> Attachment {
    Attachment {
        id: "att-fixture".into(),
        name: "shot.png".into(),
        media: "image/png".into(),
        size: 10,
        width: Some(1),
        height: Some(1),
        pages: None,
    }
}

#[test]
fn arrows_cross_each_reference_whole() {
    let source = "See @AGENTS.md and $accessibility, @\"docs/my 日本 \\\"x\\\".md\"!";
    let mut editor = draft(source);
    let chips = syntax::chips(source);
    assert_eq!(chips.len(), 3);
    let mut visited = vec![editor.cursor];
    while editor.cursor > 0 {
        press(&mut editor, LEFT, 0);
        visited.push(editor.cursor);
    }
    let expected: Vec<_> = source
        .char_indices()
        .map(|(byte, _)| byte)
        .chain([source.len()])
        .filter(|byte| {
            !chips
                .iter()
                .any(|chip| chip.start < *byte && *byte < chip.end)
        })
        .rev()
        .collect();
    assert_eq!(visited, expected);
    let mut forward = vec![0];
    while editor.cursor < source.len() {
        press(&mut editor, RIGHT, 0);
        forward.push(editor.cursor);
    }
    visited.reverse();
    assert_eq!(forward, visited);
    assert_eq!(editor.text, source);
}

#[test]
fn backspace_after_and_delete_before_remove_the_whole_reference() {
    let mut editor = draft("See @AGENTS.md, then $accessibility now");
    editor.cursor = "See @AGENTS.md".len();
    press(&mut editor, BACKSPACE, 0);
    assert_eq!(editor.text, "See , then $accessibility now");
    assert_eq!(editor.cursor, "See ".len());
    editor.cursor = "See , then ".len();
    press(&mut editor, DELETE, 0);
    assert_eq!(editor.text, "See , then  now");
    assert_eq!(editor.cursor, "See , then ".len());
    // Ordinary characters still go one at a time.
    press(&mut editor, BACKSPACE, 0);
    assert_eq!(editor.text, "See , then now");
}

#[test]
fn word_and_line_controls_treat_a_reference_as_one_word() {
    let source = "Ask $accessibility about @AGENTS.md, then stop";
    let mut editor = draft(source);
    let file = source.find('@').unwrap();
    let end = file + "@AGENTS.md".len();
    editor.cursor = end;
    press(&mut editor, LEFT, CTRL);
    assert_eq!(editor.cursor, file);
    press(&mut editor, RIGHT, CTRL);
    assert_eq!(editor.cursor, end);
    // Ctrl+Backspace and Ctrl+W remove the reference, not part of it.
    press(&mut editor, BACKSPACE, CTRL);
    assert_eq!(editor.text, "Ask $accessibility about , then stop");
    editor.cursor = "Ask $accessibility".len();
    press(&mut editor, 87, CTRL);
    assert_eq!(editor.text, "Ask  about , then stop");
    let mut editor = draft(source);
    editor.cursor = "Ask ".len();
    press(&mut editor, DELETE, CTRL);
    assert_eq!(editor.text, "Ask  about @AGENTS.md, then stop");
    // Line controls end at line edges, which are never inside a reference.
    let mut editor = draft("first\nsee @AGENTS.md now");
    press(&mut editor, HOME, 0);
    assert_eq!(editor.cursor, "first\n".len());
    press(&mut editor, END, 0);
    editor.cursor = "first\nsee @AGENTS.md".len();
    press(&mut editor, 85, CTRL);
    assert_eq!(editor.text, "first\n now");
}

#[test]
fn every_edit_and_movement_leaves_references_whole() {
    let mut mixed = draft("Look ");
    mixed.attach(image());
    mixed.compose("@AGENTS.md then $review.", false);
    let sources = [
        draft("See @AGENTS.md and $accessibility, then @\"docs/my long file.md\"!"),
        draft("Quote @\"docs/a\\\"b.md\" ok\n@\"docs/日本 notes.md\" é $review\nnext @src/main.rs"),
        mixed,
    ];
    let keys = [
        (LEFT, 0),
        (RIGHT, 0),
        (LEFT, CTRL),
        (RIGHT, CTRL),
        (BACKSPACE, 0),
        (DELETE, 0),
        (BACKSPACE, CTRL),
        (DELETE, CTRL),
        (87, CTRL),
        (85, CTRL),
        (HOME, 0),
        (END, 0),
        (UP, 0),
        (DOWN, 0),
    ];
    for source in sources {
        let before = references(&source.text);
        let chips = syntax::chips(&source.text);
        let cursors: Vec<_> = source
            .text
            .char_indices()
            .map(|(byte, _)| byte)
            .chain([source.text.len()])
            .filter(|byte| {
                !chips
                    .iter()
                    .any(|chip| chip.start < *byte && *byte < chip.end)
            })
            .collect();
        for width in [8, 20, 80] {
            for &cursor in &cursors {
                for (code, modifiers) in keys {
                    let mut editor = source.clone();
                    editor.cursor = cursor;
                    drop(editor.layout(width, true));
                    if matches!(code, UP | DOWN) {
                        editor.vertical(code == UP, width, true);
                    } else {
                        press(&mut editor, code, modifiers);
                    }
                    let context =
                        format!("{:?} {code}+{modifiers} at {cursor}, {width}", source.text);
                    for chip in syntax::chips(&editor.text) {
                        assert!(
                            !(chip.start < editor.cursor && editor.cursor < chip.end),
                            "{context}"
                        );
                    }
                    let mut remaining = before.clone();
                    for reference in references(&editor.text) {
                        let index = remaining.iter().position(|kept| *kept == reference);
                        assert!(index.is_some(), "{context}: fragment {reference}");
                        remaining.remove(index.unwrap());
                    }
                    let markers = editor.text.chars().filter(|&ch| ch == MARKER).count();
                    assert_eq!(markers, editor.attachments.len(), "{context}");
                    assert_cached_layout(&editor, width);
                }
            }
        }
    }
}

#[test]
fn text_beside_a_reference_never_changes_its_value() {
    let mut editor = draft("@AGENTS.md");
    typed(&mut editor, "x");
    assert_eq!(editor.text, "@AGENTS.md x");
    assert_eq!(editor.cursor, editor.text.len());
    let mut editor = draft("@AGENTS.md");
    editor.cursor = 0;
    typed(&mut editor, "x");
    assert_eq!(editor.text, "x @AGENTS.md");
    assert_eq!(editor.cursor, 1);
    // Punctuation leaves the reference as it is, so no space is needed.
    let mut editor = draft("$review");
    typed(&mut editor, ".5");
    assert_eq!(editor.text, "$review. 5");
    assert_eq!(references(&editor.text), ["$review"]);
    let mut editor = draft("$review");
    editor.cursor = 0;
    typed(&mut editor, "$");
    assert_eq!(editor.text, "$ $review");
    // A pasted quote cannot swallow a later reference.
    let mut editor = draft("@ and @AGENTS.md said \"hi\"");
    editor.cursor = 1;
    assert!(editor.compose("\"", false));
    assert_eq!(editor.text, "@ \" and @AGENTS.md said \"hi\"");
    assert_eq!(references(&editor.text), ["@AGENTS.md"]);
    // A `/` that makes the draft a command goes in as typed; arguments stay text.
    let mut editor = draft("x @AGENTS.md");
    editor.cursor = 0;
    typed(&mut editor, "/");
    assert_eq!(editor.text, "/x @AGENTS.md");
    assert!(shown(&editor, 80).is_empty());
}

#[test]
fn a_reference_typed_by_hand_stays_editable_until_its_word_ends() {
    let mut editor = Editor::default();
    typed(&mut editor, "Use @AGENTS.mx");
    assert!(shown(&editor, 80).is_empty());
    press(&mut editor, BACKSPACE, 0);
    typed(&mut editor, "d,");
    assert_eq!(editor.text, "Use @AGENTS.md,");
    assert!(shown(&editor, 80).is_empty());
    typed(&mut editor, " ");
    assert_eq!(shown(&editor, 80), [" @AGENTS.md "]);
    press(&mut editor, BACKSPACE, 0);
    press(&mut editor, BACKSPACE, 0);
    assert_eq!(editor.text, "Use @AGENTS.md");
    press(&mut editor, BACKSPACE, 0);
    assert_eq!(editor.text, "Use ");

    // Any movement ends the typing: the reference becomes one element.
    let mut editor = Editor::default();
    typed(&mut editor, "see $revi");
    press(&mut editor, LEFT, 0);
    assert_eq!(editor.cursor, "see ".len());
    assert_eq!(shown(&editor, 80), [" $revi "]);

    // A sigil typed before a word keeps the caret in the word being written.
    let mut editor = draft("foo");
    editor.cursor = 0;
    typed(&mut editor, "@");
    assert_eq!(editor.cursor, 1);
    assert!(shown(&editor, 80).is_empty());
    press(&mut editor, RIGHT, 0);
    assert_eq!(editor.cursor, "@foo".len());
    assert_eq!(shown(&editor, 80), [" @foo "]);

    // A reference typed before a chip is compared with the chips' moved positions.
    let mut editor = draft("$abc");
    editor.cursor = 0;
    assert!(editor.compose("$re", true));
    assert_eq!(shown(&editor, 80), [" $abc "]);
    typed(&mut editor, "v");
    assert_eq!(editor.text, "$rev $abc");

    // Pasted text is complete: its reference is a chip at once.
    let mut editor = Editor::default();
    assert!(editor.compose("see @README.md", false));
    assert_eq!(shown(&editor, 80), [" @README.md "]);
    press(&mut editor, BACKSPACE, 0);
    assert_eq!(editor.text, "see ");
}

#[test]
fn references_beside_attachments_stay_separate_elements() {
    let mut editor = draft("Look ");
    editor.attach(image());
    assert!(editor.compose("@AGENTS.md", false));
    assert_eq!(shown(&editor, 80), [" 1 Image ", " @AGENTS.md "]);
    press(&mut editor, BACKSPACE, 0);
    assert_eq!(editor.text, format!("Look {MARKER}"));
    assert_eq!(editor.attachments, [image()]);
    press(&mut editor, BACKSPACE, 0);
    assert_eq!(editor.text, "Look ");
    assert!(editor.attachments.is_empty());
}

#[test]
fn other_editors_and_literal_text_keep_character_editing() {
    let mut editor = Editor::default();
    editor.insert("Use $PATH and @team");
    let layout = editor.layout(80, false);
    assert_eq!(layout.rows[0].text, "Use $PATH and @team");
    assert!(layout.rows[0].labels.is_empty());
    editor.key(key(BACKSPACE, 0));
    assert_eq!(editor.text, "Use $PATH and @tea");
    editor.key(key(LEFT, CTRL));
    assert_eq!(editor.cursor, "Use $PATH and @".len());
    for source in [
        "$ @",
        r"Mail a@b.com US$5 \$review $5.00",
        "/model @literal $argument",
    ] {
        let mut editor = draft(source);
        assert!(shown(&editor, 80).is_empty(), "{source}");
        assert_eq!(editor.layout(80, true).rows[0].text, source);
        press(&mut editor, BACKSPACE, 0);
        press(&mut editor, LEFT, 0);
        assert_eq!(editor.text, source[..source.len() - 1]);
        assert_eq!(editor.cursor, source.len() - 2);
    }
}

#[test]
fn removing_the_only_separator_steps_over_it_instead() {
    let mut editor = draft("@AGENTS.md now");
    editor.cursor = "@AGENTS.md ".len();
    press(&mut editor, BACKSPACE, 0);
    assert_eq!(editor.text, "@AGENTS.md now");
    assert_eq!(editor.cursor, "@AGENTS.md".len());
    press(&mut editor, DELETE, 0);
    assert_eq!(editor.text, "@AGENTS.md now");
    assert_eq!(editor.cursor, "@AGENTS.md ".len());
    press(&mut editor, DELETE, 0);
    assert_eq!(editor.text, "@AGENTS.md ow");
    let mut editor = draft("x @AGENTS.md");
    editor.cursor = 2;
    press(&mut editor, BACKSPACE, 0);
    assert_eq!((editor.text.as_str(), editor.cursor), ("x @AGENTS.md", 1));
    // A draft that becomes a command keeps its text as typed.
    let mut editor = draft("x/a @AGENTS.md");
    editor.cursor = 1;
    press(&mut editor, BACKSPACE, 0);
    assert_eq!(editor.text, "/a @AGENTS.md");
}

#[test]
fn a_restored_caret_inside_a_reference_acts_from_its_start() {
    let mut editor = Editor::default();
    editor.set(Prompt::plain("See @AGENTS.md"));
    editor.cursor = "See @AGE".len();
    let layout = editor.layout(80, true);
    assert_eq!(layout.cursor, (0, "See ".len()));
    typed(&mut editor, "x");
    assert_eq!(editor.text, "See x @AGENTS.md");
    assert_eq!(references(&editor.text), ["@AGENTS.md"]);
}

fn assert_cached_layout(editor: &Editor, width: usize) {
    let labels = editor
        .attachments
        .iter()
        .enumerate()
        .map(|(index, attachment)| attachment.chip(index + 1))
        .collect::<Vec<_>>();
    let actual = editor.layout(width, true);
    let expected = Layout::new(&editor.text, editor.cursor, width, &labels, &editor.chips());
    assert_eq!(actual.cursor, expected.cursor, "{:?}", editor.text);
    assert_eq!(actual.rows.len(), expected.rows.len(), "{:?}", editor.text);
    for (actual, expected) in actual.rows.iter().zip(&expected.rows) {
        assert_eq!(actual.text, expected.text, "{:?}", editor.text);
        assert_eq!(actual.labels, expected.labels, "{:?}", editor.text);
        assert_eq!(actual.positions, expected.positions, "{:?}", editor.text);
    }
}
