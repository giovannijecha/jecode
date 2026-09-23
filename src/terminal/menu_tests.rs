use super::*;
use crate::session::{self, Event};
use std::time::SystemTime;

fn ready() -> (model::Model, session::Session) {
    let mut model = account::model(session::Model::Luna, None);
    account::event(&mut model, Event::Ready);
    (model, session::tests::ready_fixture())
}
fn type_text(model: &mut model::Model, session: &mut session::Session, text: &str) {
    account::input(model, Key::Text(text.into()), session);
}

#[test]
fn command_selection_tab_and_escape_never_submit_a_model_prompt() {
    let (mut model, mut session) = ready();
    type_text(&mut model, &mut session, "/c");
    account::input(&mut model, Key::Down, &mut session);
    account::input(&mut model, Key::Tab, &mut session);
    assert_eq!(model.editor.text, "/compact");
    account::input(&mut model, Key::Escape, &mut session);
    assert!(!model.menu.active(&model.editor.text));
    assert_eq!(model.editor.text, "/compact");
    model.editor.take();
    type_text(&mut model, &mut session, "/not-a-command");
    account::input(&mut model, Key::Enter, &mut session);
    assert!(session.ready());
    assert!(model.blocks.iter().all(|b| b.speaker != "You"));
    assert_eq!(model.editor.text, "/not-a-command");
}

#[test]
fn model_picker_filters_and_waits_for_acknowledgement() {
    let (mut model, mut session) = ready();
    type_text(&mut model, &mut session, "/model");
    account::input(&mut model, Key::Enter, &mut session);
    type_text(&mut model, &mut session, "terra");
    account::input(&mut model, Key::Enter, &mut session);
    assert_eq!(
        model.account.as_ref().unwrap().selected,
        session::Model::Luna
    );
    assert!(!session.ready());
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    while !session.ready() {
        if let Some(event) = session.poll() {
            account::event(&mut model, event);
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(
        model.account.as_ref().unwrap().selected,
        session::Model::Terra
    );
    assert!(model.menu.panel.is_none());
    assert!(model.editor.text.is_empty());
}

#[test]
fn resume_filters_captured_ids_excludes_current_and_cancel_is_inert() {
    let (mut model, mut session) = ready();
    let entry = |id: &str, title: &str| session::persistence::Listed {
        id: id.into(),
        title: title.into(),
        turns: 1,
        model: Some(session::Model::Luna),
        workspace: Some("fixture-folder".into()),
        directory: Some("fixture-folder".into()),
        modified: SystemTime::now(),
    };
    model.menu.open(menu::sessions(
        vec![
            entry("current", "Current"),
            entry("other", "Remember the test"),
        ],
        Some("current"),
    ));
    assert_eq!(model.menu.entries("").len(), 1);
    type_text(&mut model, &mut session, "fixture-folder");
    account::input(&mut model, Key::Escape, &mut session);
    assert!(model.navigation.is_none());
    assert!(model.editor.text.is_empty());
    model.menu.open(menu::sessions(
        vec![entry("other", "Remember the test")],
        None,
    ));
    type_text(&mut model, &mut session, "test");
    account::input(&mut model, Key::Enter, &mut session);
    assert!(matches!(&model.navigation, Some(navigation::Request::Resume(id)) if id == "other"));
    assert!(session.ready());
}

#[test]
fn busy_navigation_keeps_the_draft_and_menu_rows_stay_in_bounds() {
    let (mut model, mut session) = ready();
    assert!(session.submit("offline fixture"));
    type_text(&mut model, &mut session, "/new");
    account::input(&mut model, Key::Enter, &mut session);
    assert!(model.navigation.is_none());
    assert_eq!(model.editor.text, "/new");
    model.editor.take();
    model.editor.insert("/");
    model.blocks.push(model::Block {
        speaker: "Assistant",
        text: "Existing answer".into(),
    });
    for width in [1, 24, 48, 80, 120] {
        for height in [1, 9, 12, 24, 40] {
            let rows = view::chrome(&model, width, height);
            assert!(rows.len() < height || height == 1);
            assert!(
                rows.iter()
                    .all(|r| r.transient && text::width(&r.text) < width)
            );
        }
    }
    let mut decoder = input::Decoder::default();
    assert_eq!(
        decoder.push(b"\x1b[A\x1bOB", Instant::now()),
        [Key::Up, Key::Down]
    );
}
