//! Local navigation never becomes a model message or expands its tool schema.
use super::{
    Key,
    menu::{self, Action},
    model::{Block, Model},
    navigation::Request,
};
use crate::{
    session::Session,
    state::{Store, settings::Settings},
};

pub(super) fn input(model: &mut Model, key: &Key, session: &mut Session) -> bool {
    if matches!(key, Key::Text(_) | Key::Backspace | Key::Delete) {
        if (model.menu.active(&model.editor.text)
            || matches!(key, Key::Text(text) if model.editor.text.is_empty() && text.starts_with('/')))
            && let Some(view) = model.account.as_mut().filter(|view| view.ready())
        {
            view.notice.clear();
        }
        model.menu.selected = 0;
        model.menu.hidden = false;
        return false;
    }
    if model.menu.active(&model.editor.text) {
        let entries = model.menu.entries(&model.editor.text);
        let count = entries.len();
        match key {
            Key::Escape => {
                if model.menu.panel.is_some() {
                    model.editor.take();
                }
                model.menu.close();
                if session.ready() {
                    model.account.as_mut().unwrap().notice.clear();
                }
                return true;
            }
            Key::Up | Key::Down => {
                if count > 0 {
                    let selected = model.menu.selected.min(count - 1);
                    model.menu.selected = if *key == Key::Up {
                        (selected + count - 1) % count
                    } else {
                        (selected + 1) % count
                    };
                }
                return true;
            }
            Key::Tab if model.menu.panel.is_none() => {
                if let Some(entry) = entries.get(model.menu.selected.min(count.saturating_sub(1))) {
                    model.editor.text = entry.label.clone();
                    model.editor.cursor = model.editor.text.len();
                    model.menu.selected = 0;
                }
                return true;
            }
            Key::Enter => {
                if let Some(entry) = entries.get(model.menu.selected.min(count.saturating_sub(1))) {
                    execute(model, session, entry.action.clone());
                }
                return true;
            }
            _ => {}
        }
    }
    if *key == Key::Enter && model.editor.text.starts_with('/') {
        let command = model.editor.text.trim();
        if let Some(entry) = menu::commands().into_iter().find(|e| e.label == command) {
            execute(model, session, entry.action);
        } else {
            notice(model, "Unknown command · type / to choose");
        }
        return true;
    }
    false
}

fn execute(model: &mut Model, session: &mut Session, action: Action) {
    if !session.ready() || !model.account.as_ref().is_some_and(|v| v.ready()) {
        notice(model, "Wait for the current operation · Esc stops");
        return;
    }
    model.account.as_mut().unwrap().notice.clear();
    let done = match action {
        Action::New => {
            model.navigation = Some(Request::New);
            true
        }
        Action::Resume(id) => {
            model.navigation = Some(Request::Resume(id));
            true
        }
        Action::Browse => match crate::session::persistence::list() {
            Ok(sessions) => {
                let id = model.account.as_ref().and_then(|v| v.id.as_deref());
                model.menu.open(menu::sessions(sessions, id));
                true
            }
            Err(_) => {
                notice(
                    model,
                    "Cannot read saved sessions · current conversation kept",
                );
                false
            }
        },
        Action::Models => {
            let selected = model.account.as_ref().unwrap().selected;
            model.menu.open(menu::models(selected));
            true
        }
        Action::Settings => match Settings::user() {
            Ok(settings) => {
                model.menu.open(menu::settings(&settings));
                true
            }
            Err(_) => {
                notice(model, "Cannot read ~/.jecode/v1/settings.json · file kept");
                false
            }
        },
        Action::Preference(change) => {
            match Store::user().and_then(|store| Settings::update(&store, change)) {
                Ok(settings) => {
                    model.tools.reduced_motion = std::env::var_os("JECODE_REDUCED_MOTION")
                        .map_or(settings.reduced_motion, |v| !v.is_empty() && v != "0");
                    let selected = model.menu.selected;
                    model.menu.open(menu::settings(&settings));
                    model.menu.selected = selected;
                    true
                }
                Err(_) => {
                    notice(
                        model,
                        "Preferences could not be saved · check settings file and permissions",
                    );
                    false
                }
            }
        }
        Action::Model(selected) => {
            if !session.set_model(selected) {
                return;
            }
            model.menu.close();
            model.account.as_mut().unwrap().updating();
            true
        }
        Action::Context => {
            model.menu.close();
            if session.inspect_context() {
                model.account.as_mut().unwrap().inspecting();
                true
            } else {
                false
            }
        }
        Action::Compact => {
            if !session.compact() {
                return;
            }
            model.menu.close();
            super::account::compacting(model);
            true
        }
        Action::Help => {
            let commands = menu::commands()
                .into_iter()
                .map(|e| format!("{} — {}", e.label, e.description))
                .collect::<Vec<_>>()
                .join("\n");
            let view = model.account.as_ref().unwrap();
            let location = view.workspace.as_deref().unwrap_or("Conversation only");
            let access = if view.workspace.is_some() {
                view.access.name()
            } else {
                "no file tools"
            };
            model.blocks.push(Block {
                speaker: "Status",
                text: format!(
                    "{commands}\n\n↑↓ choose · Enter select · Tab complete\nEsc closes menus or stops work · Ctrl+Q saves and exits\nEnter queues guidance while a turn runs.\n\nDirectory: {location}\nAccess: {access}. Changes and commands require approval."
                ),
            });
            model.menu.close();
            true
        }
    };
    if done {
        model.editor.take();
    }
}
fn notice(model: &mut Model, text: &str) {
    if let Some(view) = &mut model.account {
        view.notice = text.into();
    }
}
