//! The composer's `$` and `@` selectors, driven through the same input as the terminal.

use super::*;
use crate::{
    agent::Agent,
    attachments::Attachment,
    backend::openrouter::OpenRouter,
    test_support::Directory,
    tools::Tools,
    tui::{Decoded, line::Line, tests, view},
};
use std::{
    fs,
    time::{Duration, Instant},
};

mod files_tests;
mod turn_tests;

const BACKSPACE: u16 = 8;
const TAB: u16 = 9;
const ENTER: u16 = 13;
const ESC: u16 = 27;
const DOWN: u16 = 40;

pub(super) fn app(home: &Directory, project: &Directory) -> App {
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let mut app = App::new(agent, tests::config(project), None);
    app.state.width = 80;
    app.state.height = 32;
    app
}

pub(super) fn skill(home: &Directory, name: &str, description: &str) {
    let package = home.path().join("skills").join(name);
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n"),
    )
    .unwrap();
}

pub(super) fn key(app: &mut App, code: u16) {
    let key = Key {
        code,
        modifiers: 0,
        character: 0,
    };
    assert!(!app.input(Decoded::Key(key)).unwrap());
}

/// Typed one key at a time, as a terminal delivers it.
pub(super) fn typed(app: &mut App, text: &str) {
    for character in text.chars() {
        assert!(!app.input(Decoded::Text(character.into())).unwrap());
    }
}

pub(super) fn live(app: &App) -> Vec<String> {
    view::frame(&app.state, "fixture/model", "fixture")
        .live
        .iter()
        .map(Line::plain)
        .collect()
}

/// The selector's rows, in order.
pub(super) fn listed(app: &App) -> Vec<String> {
    let menu = app.state.selector.as_ref().expect("a selector");
    menu.filtered
        .iter()
        .map(|hit| menu.options[hit.index].name.clone())
        .collect()
}

pub(super) fn focused(app: &App) -> String {
    let menu = app.state.selector.as_ref().unwrap();
    menu.options[menu.filtered[menu.selected].index]
        .name
        .clone()
}

fn skills_open(app: &App) -> bool {
    app.state
        .selector
        .as_ref()
        .is_some_and(|menu| matches!(menu.purpose, Purpose::Skills { .. }))
}

/// Nothing went to the agent: no turn, no queued message, no prompt history.
pub(super) fn nothing_sent(app: &App) {
    assert!(app.worker.is_none());
    assert!(app.state.queue.is_empty());
    assert!(app.state.history.snapshot().is_empty());
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
fn dollar_opens_a_filterable_skill_sheet_over_the_kept_draft() {
    let (home, project) = (Directory::new(), Directory::new());
    skill(&home, "review", "Inspect a change for defects");
    skill(&home, "release", "Prepare a release");
    let mut app = app(&home, &project);
    typed(&mut app, "please $");
    assert!(skills_open(&app));
    assert_eq!(app.state.editor.text, "please $");
    assert_eq!(listed(&app), ["release", "review"]);

    let rows = live(&app);
    let at = |needle: &str| rows.iter().position(|row| row.contains(needle));
    let (kept, title, field) = (
        at("draft kept").expect("the kept draft"),
        at("─ Skills").expect("the title"),
        at("  Filter").expect("the filter"),
    );
    assert!(rows[kept].starts_with("› please $"), "{rows:#?}");
    assert_eq!((title, field), (kept + 2, kept + 3), "{rows:#?}");
    assert!(rows[title].contains("2 skills"), "{rows:#?}");
    assert!(
        rows.iter()
            .any(|row| row.contains("Inspect a change for defects"))
    );
    // The composer is gone; the filter holds the one caret.
    assert!(!rows.iter().any(|row| row.contains("Ask anything")));
    let frame = view::frame(&app.state, "fixture/model", "fixture");
    assert_eq!(frame.cursor.map(|(row, _)| row), Some(field));
    assert!(rows.iter().any(|row| row.contains("Enter insert")));

    // Two skills are still filtered by typing, digits included.
    typed(&mut app, "1");
    assert!(listed(&app).is_empty());
    assert!(
        live(&app)
            .iter()
            .any(|row| row.contains("No matching skill"))
    );
    assert!(!live(&app).iter().any(|row| row.contains("Enter insert")));
    key(&mut app, ENTER);
    key(&mut app, TAB);
    assert!(skills_open(&app), "an empty result selects nothing");
    assert_eq!(app.state.editor.text, "please $");
    key(&mut app, BACKSPACE);
    typed(&mut app, "rev");
    assert_eq!(listed(&app), ["review"]);
    assert!(live(&app).iter().any(|row| row.contains("1 of 2 skills")));
    nothing_sent(&app);
}

#[test]
fn a_chosen_skill_replaces_the_sigil_mid_draft_and_keeps_attachments() {
    let (home, project) = (Directory::new(), Directory::new());
    skill(&home, "review", "Inspect a change");
    let mut app = app(&home, &project);
    app.state.editor.insert("fix ");
    app.state.editor.attach(image());
    app.state.editor.insert(" then ");
    let cursor = app.state.editor.cursor;
    app.state.editor.insert(" the tests");
    app.state.editor.cursor = cursor;
    typed(&mut app, "$");
    assert!(skills_open(&app));
    key(&mut app, ENTER);
    assert!(app.state.selector.is_none());
    let text = format!("fix {} then $review the tests", crate::attachments::MARKER);
    assert_eq!(app.state.editor.text, text);
    // After the reference and the space that was already there.
    assert_eq!(
        &app.state.editor.text[app.state.editor.cursor..],
        "the tests"
    );
    assert_eq!(app.state.editor.attachments, [image()]);
    nothing_sent(&app);
}

#[test]
fn tab_chooses_and_esc_returns_the_draft_as_it_opened() {
    let (home, project) = (Directory::new(), Directory::new());
    skill(&home, "review", "Inspect a change");
    let mut app = app(&home, &project);
    typed(&mut app, "$");
    let opened = app.state.editor.clone();
    typed(&mut app, "rev");
    key(&mut app, DOWN);
    key(&mut app, ESC);
    assert!(app.state.selector.is_none());
    assert_eq!(app.state.editor, opened);
    // The sigil stays text and typing continues in the draft: a shell variable.
    typed(&mut app, "HOME ");
    assert!(app.state.selector.is_none());
    assert_eq!(app.state.editor.text, "$HOME ");
    typed(&mut app, "$");
    key(&mut app, TAB);
    assert_eq!(app.state.editor.text, "$HOME $review ");
    assert_eq!(app.state.editor.cursor, app.state.editor.text.len());
    nothing_sent(&app);
}

#[test]
fn tab_on_an_existing_reference_replaces_it_where_it_stands() {
    let (home, project) = (Directory::new(), Directory::new());
    skill(&home, "release", "Prepare a release");
    skill(&home, "review", "Inspect a change");
    let mut app = app(&home, &project);
    app.state.editor.insert("use $relase now");
    app.state.editor.cursor = 7;
    app.edited();
    key(&mut app, TAB);
    assert!(skills_open(&app));
    assert_eq!(app.state.selector.as_ref().unwrap().editor.text, "relase");
    assert_eq!(focused(&app), "release");
    key(&mut app, ENTER);
    assert_eq!(app.state.editor.text, "use $release now");
    assert_eq!(&app.state.editor.text[app.state.editor.cursor..], "now");
    // Tab elsewhere in an ordinary draft stays a no-op.
    app.state.editor.cursor = 1;
    key(&mut app, TAB);
    assert!(app.state.selector.is_none());
    assert_eq!(app.state.editor.text, "use $release now");
}

#[test]
fn literal_symbols_typed_or_pasted_stay_text() {
    let (home, project) = (Directory::new(), Directory::new());
    skill(&home, "review", "Inspect a change");
    let mut app = app(&home, &project);
    typed(&mut app, r"mail a@b.com, pay US$5 or \$x");
    assert!(app.state.selector.is_none());
    assert_eq!(app.state.editor.text, r"mail a@b.com, pay US$5 or \$x");
    app.state.editor.take();
    let code = "echo $HOME\nfn main() { let a = @x; }\n$ cargo test @ user@example.com";
    assert!(!app.input(Decoded::Paste(code.into())).unwrap());
    assert!(app.state.selector.is_none());
    assert_eq!(app.state.editor.text, code);
    // A slash command's arguments are not message text.
    app.state.editor.take();
    app.edited();
    typed(&mut app, "/resume $");
    assert!(app.state.selector.is_none());
    assert_eq!(app.state.editor.text, "/resume $");
}

#[test]
fn replacing_a_reference_preserves_following_punctuation_and_newlines() {
    let (home, project) = (Directory::new(), Directory::new());
    skill(&home, "review", "Inspect a change");
    for draft in ["use $review, then continue", "use $review\nnext line"] {
        let mut app = app(&home, &project);
        app.state.editor.insert(draft);
        app.state.editor.cursor = 7;
        app.edited();
        key(&mut app, TAB);
        assert!(skills_open(&app));
        key(&mut app, ENTER);
        assert_eq!(app.state.editor.text, draft);
        assert_eq!(app.state.editor.cursor, "use $review".len());
        nothing_sent(&app);
    }
}

#[test]
fn empty_malformed_and_refreshed_catalogs_are_explained() {
    let (home, project) = (Directory::new(), Directory::new());
    let mut app = app(&home, &project);
    typed(&mut app, "$");
    assert!(skills_open(&app));
    let rows = live(&app);
    assert!(
        rows.iter()
            .any(|row| row.contains("No skills in ~/.jecode/skills"))
    );
    assert!(
        rows.iter().any(|row| row.trim() == "Esc cancel"),
        "{rows:#?}"
    );
    key(&mut app, ENTER);
    assert!(skills_open(&app));
    key(&mut app, ESC);
    assert_eq!(app.state.editor.text, "$");

    let broken = home.path().join("skills/broken");
    fs::create_dir_all(&broken).unwrap();
    fs::write(broken.join("SKILL.md"), "not frontmatter").unwrap();
    app.state.editor.take();
    typed(&mut app, "$");
    let rows = live(&app);
    assert!(rows.iter().any(|row| row.contains("No valid skills")));
    assert!(
        rows.iter()
            .any(|row| row.contains("1 skipped · /skills shows why"))
    );
    key(&mut app, ESC);

    // Each opening reads the catalog again.
    skill(&home, "review", "Inspect a change");
    app.state.editor.take();
    typed(&mut app, "$");
    assert_eq!(listed(&app), ["review"]);
    let rows = live(&app);
    assert!(
        rows.iter().any(|row| row.contains("1 skipped")),
        "{rows:#?}"
    );
    nothing_sent(&app);
}

#[test]
fn slash_completion_and_the_commands_key_keep_their_behaviour() {
    let (home, project) = (Directory::new(), Directory::new());
    skill(&home, "review", "Inspect a change");
    let mut app = app(&home, &project);
    typed(&mut app, "/mo");
    assert_eq!(app.state.suggestions.chosen(), Some("/model"));
    key(&mut app, TAB);
    assert_eq!(app.state.editor.text, "/model");
    app.state.editor.take();
    app.edited();

    typed(&mut app, "$");
    key(&mut app, ENTER);
    let draft = app.state.editor.clone();
    assert_eq!(draft.text, "$review ");
    let ctrl_g = Key {
        code: 71,
        modifiers: 4,
        character: 0,
    };
    app.input(Decoded::Key(ctrl_g)).unwrap();
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        Purpose::Commands { .. }
    ));
    key(&mut app, ESC);
    assert!(app.state.selector.is_none());
    assert_eq!(app.state.editor, draft);
}

/// Waits for the open file walk to finish delivering.
pub(super) fn settle(app: &mut App) {
    let started = Instant::now();
    loop {
        app.poll().unwrap();
        if !app.references_pending() {
            app.poll().unwrap();
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the walk stalled"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
