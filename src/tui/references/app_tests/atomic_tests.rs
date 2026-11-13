//! Reference chips through the composer's real input: keys, Tab, history, restored
//! drafts and the fields that keep ordinary editing.

use super::*;
use crate::{ask::Request, attachments::Prompt, json, tui::ask::Panel, tui::theme};

const LEFT: u16 = 37;
const RIGHT: u16 = 39;
const UP: u16 = 38;
const HOME: u16 = 36;
const DELETE: u16 = 46;

fn ctrl(app: &mut App, code: u16) {
    let key = Key {
        code,
        modifiers: 4,
        character: 0,
    };
    assert!(!app.input(Decoded::Key(key)).unwrap());
}

fn paste(app: &mut App, text: &str) {
    assert!(!app.input(Decoded::Paste(text.into())).unwrap());
}

fn after_caret(app: &App) -> &str {
    &app.state.editor.text[app.state.editor.cursor..]
}

/// The filled chips drawn on screen, with their padding as plain spaces.
pub(super) fn drawn(app: &App) -> Vec<String> {
    view::frame(&app.state, "fixture/model", "fixture")
        .live
        .iter()
        .flat_map(|line| &line.spans)
        .filter(|span| span.style == theme::LIT && span.background == Some(theme::BADGE_IDLE))
        .map(|span| span.text.replace('\u{a0}', " "))
        .collect()
}

#[test]
fn composer_keys_move_across_and_delete_whole_references() {
    let (home, project) = (Directory::new(), Directory::new());
    let mut app = app(&home, &project);
    paste(&mut app, "Check @AGENTS.md and $accessibility now");
    assert_eq!(drawn(&app), [" @AGENTS.md ", " $accessibility "]);
    for _ in 0.." now".len() {
        key(&mut app, LEFT);
    }
    assert_eq!(after_caret(&app), " now");
    key(&mut app, LEFT);
    assert_eq!(after_caret(&app), "$accessibility now");
    key(&mut app, RIGHT);
    assert_eq!(after_caret(&app), " now");
    key(&mut app, BACKSPACE);
    assert_eq!(app.state.editor.text, "Check @AGENTS.md and  now");
    assert_eq!(drawn(&app), [" @AGENTS.md "]);
    key(&mut app, HOME);
    for _ in 0.."Check ".len() {
        key(&mut app, RIGHT);
    }
    assert_eq!(after_caret(&app), "@AGENTS.md and  now");
    key(&mut app, DELETE);
    assert_eq!(app.state.editor.text, "Check  and  now");
    assert!(drawn(&app).is_empty());

    // Word controls take a reference as one word.
    app.state.editor.take();
    paste(&mut app, "first line\nsee @AGENTS.md, $review here");
    ctrl(&mut app, LEFT);
    ctrl(&mut app, LEFT);
    assert_eq!(after_caret(&app), "$review here");
    // Removing `, ` would join both references: the caret steps over it instead.
    ctrl(&mut app, BACKSPACE);
    assert_eq!(after_caret(&app), ", $review here");
    ctrl(&mut app, BACKSPACE);
    assert_eq!(app.state.editor.text, "first line\nsee , $review here");
    assert_eq!(after_caret(&app), ", $review here");

    // Vertical movement into a chip's columns lands on its nearer end.
    app.state.editor.take();
    paste(&mut app, "abcdefghijklmnopqrstu\nsee @AGENTS.md");
    ctrl(&mut app, HOME);
    for _ in 0..7 {
        key(&mut app, RIGHT);
    }
    key(&mut app, DOWN);
    assert_eq!(after_caret(&app), "@AGENTS.md");
    nothing_sent(&app);
}

#[test]
fn tab_reopens_a_reference_from_either_end_and_esc_restores_the_draft() {
    let (home, project) = (Directory::new(), Directory::new());
    skill(&home, "release", "Prepare a release");
    skill(&home, "review", "Inspect a change");
    let mut app = app(&home, &project);
    app.state.editor.insert("fix ");
    app.state.editor.attach(image());
    assert!(app.state.editor.compose(" with $relase now", false));
    app.edited();
    let start = app.state.editor.text.find('$').unwrap();
    let end = start + "$relase".len();
    for cursor in [start, end] {
        app.state.editor.cursor = cursor;
        let before = app.state.editor.clone();
        key(&mut app, TAB);
        assert!(skills_open(&app), "{cursor}");
        assert_eq!(app.state.selector.as_ref().unwrap().editor.text, "relase");
        key(&mut app, ESC);
        assert_eq!(app.state.editor, before);
        assert_eq!(app.state.editor.attachments, [image()]);
    }
    key(&mut app, TAB);
    key(&mut app, ENTER);
    let text = format!("fix {} with $release now", crate::attachments::MARKER);
    assert_eq!(app.state.editor.text, text);
    assert_eq!(after_caret(&app), "now");
    // Removing the only separator would join the reference to `now`: the caret steps.
    key(&mut app, BACKSPACE);
    assert_eq!(app.state.editor.text, text);
    assert_eq!(after_caret(&app), " now");
    key(&mut app, BACKSPACE);
    assert_eq!(
        app.state.editor.text,
        format!("fix {} with  now", crate::attachments::MARKER)
    );
    assert_eq!(app.state.editor.attachments, [image()]);
    nothing_sent(&app);
}

#[test]
fn recalled_and_restored_drafts_keep_references_whole() {
    let (home, project) = (Directory::new(), Directory::new());
    let mut app = app(&home, &project);
    app.state.history.record(Prompt::plain("see @AGENTS.md"));
    ctrl(&mut app, 80);
    assert_eq!(app.state.editor.text, "see @AGENTS.md");
    assert_eq!(drawn(&app), [" @AGENTS.md "]);
    key(&mut app, BACKSPACE);
    assert_eq!(app.state.editor.text, "see ");

    app.state.history.cancel(&mut app.state.editor);
    app.state.editor.take();
    app.edited();
    let mut paused = Editor::default();
    paused.set(Prompt::plain("draft $review here"));
    // A caret saved inside a reference acts from its start.
    paused.cursor = "draft $re".len();
    app.state.queue.paused = Some(paused);
    key(&mut app, UP);
    assert_eq!(app.state.editor.text, "draft $review here");
    key(&mut app, RIGHT);
    assert_eq!(after_caret(&app), " here");
    assert_eq!(drawn(&app), [" $review "]);
    key(&mut app, LEFT);
    key(&mut app, LEFT);
    assert_eq!(after_caret(&app), " $review here");
}

#[test]
fn references_typed_by_hand_and_text_typed_beside_chips() {
    let (home, project) = (Directory::new(), Directory::new());
    skill(&home, "review", "Inspect a change");
    let mut app = app(&home, &project);
    typed(&mut app, "$");
    assert!(skills_open(&app));
    key(&mut app, ESC);
    typed(&mut app, "reviex");
    key(&mut app, BACKSPACE);
    typed(&mut app, "w,");
    assert_eq!(app.state.editor.text, "$review,");
    assert!(drawn(&app).is_empty(), "plain while it is typed");
    typed(&mut app, " ");
    assert_eq!(drawn(&app), [" $review "]);
    paste(&mut app, "@AGENTS.md");
    typed(&mut app, "x");
    assert_eq!(app.state.editor.text, "$review, @AGENTS.md x");
    key(&mut app, LEFT);
    key(&mut app, LEFT);
    key(&mut app, LEFT);
    assert_eq!(after_caret(&app), "@AGENTS.md x");
    typed(&mut app, "(");
    assert_eq!(app.state.editor.text, "$review, ( @AGENTS.md x");
    assert_eq!(drawn(&app), [" $review ", " @AGENTS.md "]);
    nothing_sent(&app);
}

fn question() -> Request {
    Request::parse(
        "ask",
        &json::parse(
            r#"{"questions":[{"id":"file","title":"File","question":"Which file?",
            "options":[{"id":"a","label":"A"}]}]}"#,
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn filters_and_ask_answers_keep_character_editing() {
    let (home, project) = (Directory::new(), Directory::new());
    skill(&home, "review", "Inspect a change");
    let mut app = app(&home, &project);
    typed(&mut app, "$");
    typed(&mut app, "x @AGENTS.md");
    key(&mut app, BACKSPACE);
    assert_eq!(
        app.state.selector.as_ref().unwrap().editor.text,
        "x @AGENTS.m"
    );
    key(&mut app, ESC);
    app.state.editor.take();

    let request = question();
    app.state.ask_panel = Some(Panel::new(&request, None));
    app.state.question = Some(request);
    paste(&mut app, "see @AGENTS.md");
    key(&mut app, BACKSPACE);
    key(&mut app, LEFT);
    let editor = &app.state.ask_panel.as_ref().unwrap().entry().editor;
    assert_eq!(editor.text, "see @AGENTS.m");
    assert_eq!(editor.cursor, "see @AGENTS.".len());
    assert!(app.state.editor.text.is_empty());
}
