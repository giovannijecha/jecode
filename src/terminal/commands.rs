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
            && let Some(view) = model.account.as_mut()
        {
            view.local_notice.clear();
            view.local_failed = false;
            if view.ready() {
                view.notice.clear();
            }
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
                    let view = model.account.as_mut().unwrap();
                    view.local_notice.clear();
                    view.local_failed = false;
                    if view.ready() {
                        view.notice.clear();
                    }
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
    if matches!(action, Action::Login) {
        if session.signed_out() && session.login() {
            model.account.as_mut().unwrap().logging_in();
        } else if session.ready() {
            notice(model, "Already signed in");
        } else {
            notice(model, "Wait for the current account transition");
        }
        model.menu.close();
        if model.editor.text.starts_with('/') {
            model.editor.take();
        }
        return;
    }
    if matches!(action, Action::Logout) {
        if session.logout() {
            model.account.as_mut().unwrap().signing_out();
        } else {
            notice(
                model,
                "Sign-out is already running or this session is closed",
            );
        }
        model.menu.close();
        if model.editor.text.starts_with('/') {
            model.editor.take();
        }
        return;
    }
    if (!session.ready() && !session.signed_out())
        || (!model.account.as_ref().is_some_and(|v| v.ready())
            && !model.account.as_ref().is_some_and(|v| v.signed_out()))
    {
        notice(model, "Wait for the current operation");
        return;
    }
    if session.signed_out()
        && matches!(action, Action::Model(_) | Action::Context | Action::Compact)
    {
        notice(model, "Sign in with /login before using this command");
        return;
    }
    let view = model.account.as_mut().unwrap();
    view.notice.clear();
    view.local_notice.clear();
    view.local_failed = false;
    let done = match action {
        Action::Login | Action::Logout => unreachable!(),
        Action::New => {
            model.navigation = Some(Request::New);
            true
        }
        Action::Resume(id) => {
            model.navigation = Some(Request::Resume(id));
            true
        }
        Action::Browse => match crate::session::scope::Directory::open(std::path::Path::new(
            model
                .account
                .as_ref()
                .unwrap()
                .directory
                .as_deref()
                .unwrap_or(""),
        ))
        .and_then(|directory| crate::session::persistence::list_in(&directory))
        {
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
            let location = view.directory.as_deref().unwrap_or("Conversation only");
            let access = if view.file_tools {
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
        view.local_notice = text.into();
        view.local_failed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{self, Event};
    use crate::terminal::{account, style::Tone, view};

    #[test]
    fn local_account_commands_keep_draft_history_and_runtime_regions() {
        let mut session = session::tests::ready_fixture();
        let mut model = account::model(session::Model::Luna, None);
        account::event(&mut model, Event::Ready);
        model.blocks.push(Block {
            speaker: "Assistant",
            text: "Saved answer".into(),
        });
        model.editor.insert("unsent draft");
        let before = model.blocks.len();
        execute(&mut model, &mut session, Action::Login);
        assert_eq!(
            model.account.as_ref().unwrap().local_notice,
            "Already signed in"
        );
        execute(&mut model, &mut session, Action::Logout);
        assert_eq!(model.editor.text, "unsent draft");
        assert!(matches!(
            session::tests::next(&mut session),
            Event::LoggedOut
        ));
        account::event(&mut model, Event::LoggedOut);
        assert_eq!(model.blocks.len(), before);
        account::input(&mut model, crate::terminal::Key::Enter, &mut session);
        assert_eq!(model.editor.text, "unsent draft");
        assert!(
            model
                .account
                .as_ref()
                .unwrap()
                .notice
                .contains("draft kept")
        );
        let rows = view::chrome(&model, 80, 24);
        let upper = rows
            .iter()
            .position(|row| row.text.starts_with('─'))
            .unwrap();
        let lower = rows
            .iter()
            .rposition(|row| row.text.starts_with('─'))
            .unwrap();
        assert!(
            rows[..upper]
                .iter()
                .any(|row| row.text.contains("Signed out"))
        );
        assert!(
            !rows[lower + 1..]
                .iter()
                .any(|row| row.text.contains("Signed out"))
        );
        assert!(
            rows[upper..lower]
                .iter()
                .any(|row| row.text.contains("unsent draft"))
        );
        assert!(rows.iter().all(|row| row.tone != Tone::Error));
        execute(&mut model, &mut session, Action::Login);
        assert!(matches!(
            session::tests::next(&mut session),
            Event::LoginCode(_)
        ));
        account::event(&mut model, Event::Ready);
        assert!(matches!(session::tests::next(&mut session), Event::Ready));
        assert_eq!(model.editor.text, "unsent draft");
        assert_eq!(model.blocks.len(), before);
        assert!(
            session.poll().is_none(),
            "draft must not be sent after login"
        );
    }
}
