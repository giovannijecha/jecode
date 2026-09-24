//! One in-memory saved draft while a withdrawn message is edited.
use super::{editor::Editor, menu::Menu, model::Model, prompt_history::Navigation};
use crate::session::Session;

pub(super) struct SavedDraft {
    editor: Editor,
    menu: Menu,
    navigation: Navigation,
}
impl SavedDraft {
    fn capture(model: &Model) -> Self {
        Self {
            editor: model.editor.clone(),
            menu: model.menu.clone(),
            navigation: model.prompt_history.navigation(),
        }
    }
    fn restore(self, model: &mut Model) {
        let columns = model.editor.columns();
        model.editor = self.editor;
        model.editor.set_columns(columns);
        model.menu = self.menu;
        model.prompt_history.restore_navigation(self.navigation);
    }
}

pub(super) fn retrieve(model: &mut Model, session: &mut Session) {
    if model.menu.active(&model.editor.text) {
        return;
    }
    if model
        .account
        .as_ref()
        .is_some_and(|view| view.recovery.is_some())
    {
        let view = model.account.as_mut().unwrap();
        view.local_notice = "Finish or abandon this recovered edit first".into();
        view.local_failed = false;
        return;
    }
    let Some(text) = session.withdraw_latest() else {
        let view = model.account.as_mut().unwrap();
        view.local_notice = "No pending message to edit; draft kept".into();
        view.local_failed = false;
        return;
    };
    let saved = SavedDraft::capture(model);
    let view = model.account.as_mut().unwrap();
    view.recovery = Some(saved);
    view.local_notice = "Editing withdrawn message · Enter sends · Alt+↓ abandons".into();
    view.local_failed = false;
    model.editor.replace(&text);
    model.menu = Menu::default();
    model.menu.pasted_literal = true;
}

/// Only a successful normal submission consumes the saved draft.
pub(super) fn submitted(model: &mut Model, prompt: &str, new_turn: bool) {
    let saved = model.account.as_mut().and_then(|view| view.recovery.take());
    if new_turn {
        if saved.is_some() {
            model.prompt_history.record_new_turn(prompt);
        } else {
            model.prompt_history.record(prompt);
        }
    }
    if let Some(saved) = saved {
        saved.restore(model);
    } else {
        model.menu.pasted_literal = false;
    }
}

/// Explicitly discard the visible recovered edit; never requeue it implicitly.
pub(super) fn abandon(model: &mut Model) {
    if model.menu.active(&model.editor.text) {
        return;
    }
    let saved = model.account.as_mut().and_then(|view| view.recovery.take());
    if let Some(saved) = saved {
        saved.restore(model);
        let view = model.account.as_mut().unwrap();
        view.local_notice = "Recovered edit discarded; previous draft restored".into();
        view.local_failed = false;
    }
}
