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
                } else {
                    notice(model, "No matching command or conversation · Esc closes");
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
    if !matches!(action, Action::Help | Action::Quit) && !session.ready() {
        notice(
            model,
            "Wait for the current operation · draft kept · Esc stops",
        );
        return;
    }
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
                    notice(
                        model,
                        "Preferences saved · model and access defaults apply to new conversations",
                    );
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
            session.inspect_context()
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
            model.blocks.push(Block { speaker: "Status", text: format!("{commands}\n\nType /, then ↑↓ and Enter to choose. Tab completes. Esc closes a menu or stops a running turn.\nWhile a turn runs, Enter queues your message for the next model boundary. Ctrl+Q saves and exits.") });
            model.menu.close();
            true
        }
        Action::Quit => {
            model.quit = true;
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
