use super::*;
use crate::session::{self, Event};
use std::time::SystemTime;

fn ready() -> (model::Model, session::Session) {
    let mut model = account::model(session::Model::Luna, None);
    let catalog = crate::providers::openai_account::catalog::Catalog::parse(br#"{"models":[{"slug":"gpt-5.6-luna","visibility":"list","default_reasoning_level":"medium","supported_reasoning_levels":[{"effort":"medium"}]},{"slug":"gpt-5.6-terra","visibility":"list","default_reasoning_level":"medium","supported_reasoning_levels":[{"effort":"medium"}]}]}"#).unwrap();
    account::event(&mut model, Event::CatalogLoaded(catalog));
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
    assert_eq!(model.editor.text, "/context");
    account::input(&mut model, Key::Escape, &mut session);
    assert!(!model.menu.active(&model.editor.text));
    assert_eq!(model.editor.text, "/context");
    model.editor.take();
    type_text(&mut model, &mut session, "/not-a-command");
    account::input(&mut model, Key::Enter, &mut session);
    assert!(session.ready());
    assert!(model.blocks.iter().all(|b| b.speaker != "You"));
    let receipt = model.command_receipts.values().last().unwrap();
    assert_eq!(receipt.input, "/not-a-command");
    assert_eq!(receipt.status, lab::model::Status::Failed);
    assert!(model.editor.text.is_empty());
}

#[test]
fn model_picker_filters_and_waits_for_acknowledgement() {
    let (mut model, mut session) = ready();
    type_text(&mut model, &mut session, "/model");
    account::input(&mut model, Key::Enter, &mut session);
    type_text(&mut model, &mut session, "2"); // Numbered short model menu.
    assert_eq!(
        model.account.as_ref().unwrap().selected,
        session::Model::Luna
    );
    assert!(session.ready());
    type_text(&mut model, &mut session, "2"); // Medium effort.
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
    let receipt = model.command_receipts.values().last().unwrap();
    assert_eq!(receipt.input, "/model gpt-5.6-terra");
    assert_eq!(receipt.result, "Model set to gpt-5.6-terra");
    assert_eq!(receipt.note, "was gpt-5.6-luna");
}

#[test]
fn argument_commands_report_actual_success_and_keep_previous_value_on_no_match() {
    let (mut model, mut session) = ready();
    type_text(&mut model, &mut session, "/model terra");
    account::input(&mut model, Key::Enter, &mut session);
    let receipt = model.command_receipts.values().last().unwrap();
    assert_eq!(receipt.input, "/model terra");
    assert_eq!(receipt.status, lab::model::Status::Running);
    assert_eq!(
        model.account.as_ref().unwrap().selected,
        session::Model::Luna
    );
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    while !session.ready() {
        if let Some(event) = session.poll() {
            account::event(&mut model, event);
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let receipt = model.command_receipts.values().last().unwrap();
    assert_eq!(receipt.status, lab::model::Status::Done);
    assert_eq!(receipt.result, "Model set to gpt-5.6-terra");
    assert_eq!(receipt.note, "was gpt-5.6-luna");
    assert_eq!(
        model.account.as_ref().unwrap().selected,
        session::Model::Terra
    );

    type_text(&mut model, &mut session, "/effort xhigh");
    account::input(&mut model, Key::Enter, &mut session);
    let receipt = model.command_receipts.values().last().unwrap();
    assert_eq!(receipt.status, lab::model::Status::Warned);
    assert!(receipt.result.contains("No effort matches"));
    assert_eq!(receipt.note, "kept medium");
    assert_eq!(
        model.account.as_ref().unwrap().selected,
        session::Model::Terra
    );
    assert!(session.ready());

    type_text(&mut model, &mut session, "/effort provider default");
    account::input(&mut model, Key::Enter, &mut session);
    assert!(!session.ready());
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    while !session.ready() {
        if let Some(event) = session.poll() {
            account::event(&mut model, event);
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let receipt = model.command_receipts.values().last().unwrap();
    assert_eq!(receipt.input, "/effort provider default");
    assert_eq!(receipt.status, lab::model::Status::Done);
    assert_eq!(receipt.result, "Effort set to provider default");
    assert_eq!(receipt.note, "was medium");
    assert_eq!(model.account.as_ref().unwrap().selected.effort(), None);
}

#[test]
fn status_and_clear_use_the_selected_session_without_sending_model_input() {
    let (mut model, mut session) = ready();
    type_text(&mut model, &mut session, "/status");
    account::input(&mut model, Key::Enter, &mut session);
    let receipt = model.command_receipts.values().last().unwrap();
    assert_eq!(receipt.input, "/status");
    assert!(
        receipt
            .facts
            .iter()
            .any(|(name, value)| name == "model" && value == "gpt-5.6-luna")
    );
    assert!(session.ready());
    assert!(model.blocks.iter().all(|block| block.speaker != "You"));
    type_text(&mut model, &mut session, "/clear");
    account::input(&mut model, Key::Enter, &mut session);
    assert!(matches!(model.navigation, Some(navigation::Request::Clear)));
    assert!(session.ready());
}

#[test]
fn cancelling_effort_step_leaves_pair_and_draft_unchanged() {
    let (mut model, mut session) = ready();
    type_text(&mut model, &mut session, "/model");
    account::input(&mut model, Key::Enter, &mut session);
    type_text(&mut model, &mut session, "2");
    assert_eq!(model.menu.panel.as_ref().unwrap().title, "Reasoning effort");
    account::input(&mut model, Key::Escape, &mut session);
    assert_eq!(
        model.account.as_ref().unwrap().selected,
        session::Model::Luna
    );
    assert!(session.ready());
    assert!(model.menu.panel.is_none());
    assert!(model.editor.text.is_empty());
}

#[test]
fn settings_model_and_effort_are_one_default_update_action() {
    let settings = crate::state::settings::Settings::default();
    let panel = menu::settings(&settings);
    assert!(matches!(
        panel.entries[0].action,
        menu::Action::DefaultModels
    ));
    let catalog = crate::providers::openai_account::catalog::Catalog::parse(br#"{"models":[{"slug":"first","visibility":"list","default_reasoning_level":"low","supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}]},{"slug":"second","visibility":"list","supported_reasoning_levels":[{"effort":"medium"}]}]}"#).unwrap();
    let models = menu::models(&catalog, settings.model, true);
    assert_eq!(models.entries.len(), 2);
    let effort_panel = menu::efforts(catalog.entry("first").unwrap(), settings.model, true);
    assert_eq!(effort_panel.entries.len(), 3);
    assert!(
        matches!(effort_panel.entries[0].action, menu::Action::Preference(crate::state::settings::Change::Model(selection)) if selection.id() == "first" && selection.effort().is_none())
    );
    assert!(
        matches!(effort_panel.entries[2].action, menu::Action::Preference(crate::state::settings::Change::Model(selection)) if selection.id() == "first" && selection.effort() == Some("high"))
    );
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
