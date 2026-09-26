//! Local navigation never becomes a model message or expands its tool schema.
//! Argument resolution, dispatch and acknowledgement share this boundary so
//! a receipt cannot report a choice before the session saves that choice.
use super::{
    Key,
    menu::{self, Action},
    model::{Block, Model},
    navigation::Request,
};
use crate::{
    session::{self, Session},
    state::{Store, settings::Settings},
};

pub(super) fn input(model: &mut Model, key: &Key, session: &mut Session) -> bool {
    if model.menu.panel.is_some() {
        match key {
            Key::Text(text) => {
                if model
                    .menu
                    .panel
                    .as_ref()
                    .is_some_and(|panel| panel.entries.len() <= super::lab::picker::VISIBLE)
                {
                    if let Some(index) = (text.chars().count() == 1)
                        .then(|| text.chars().next().and_then(|ch| ch.to_digit(10)))
                        .flatten()
                        .map(|n| n as usize)
                        .filter(|n| *n > 0 && *n <= model.menu.entries("").len())
                    {
                        let entry = model.menu.entries("")[index - 1].clone();
                        execute(model, session, entry.action);
                        return true;
                    }
                    return true;
                }
                model.menu.query.push_str(text);
                model.menu.selected = 0;
                return true;
            }
            Key::Paste(text) => {
                model.menu.query.push_str(text);
                model.menu.selected = 0;
                return true;
            }
            Key::Backspace => {
                model.menu.query.pop();
                model.menu.selected = 0;
                return true;
            }
            _ => {}
        }
    }
    if matches!(
        key,
        Key::Text(_)
            | Key::Paste(_)
            | Key::Backspace
            | Key::Delete
            | Key::WordBackspace
            | Key::WordDelete
    ) {
        if model.editor.text.is_empty()
            && model
                .account
                .as_ref()
                .is_none_or(|view| view.recovery.is_none())
        {
            model.menu.pasted_literal = false;
        }
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
    if *key == Key::Enter
        && model.menu.panel.is_none()
        && model.editor.text.starts_with('/')
        && !model.editor.text.contains('\n')
        && !model.menu.pasted_literal
    {
        let line = model.editor.text.trim().to_owned();
        if let Some((name, args)) = line.split_once(char::is_whitespace)
            && !args.trim().is_empty()
        {
            match name {
                "/model" | "/effort" => {
                    execute_argument(model, session, &line);
                    return true;
                }
                _ => {}
            }
        }
    }
    if model.menu.active(&model.editor.text) {
        let entries = model.menu.entries(&model.editor.text);
        let count = entries.len();
        match key {
            Key::Escape => {
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
                        selected.saturating_sub(1)
                    } else {
                        (selected + 1).min(count - 1)
                    };
                }
                return true;
            }
            Key::PageUp | Key::PageDown | Key::HistoryPrevious | Key::HistoryNext => return true,
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
                } else if model.menu.panel.is_none() && model.editor.text.starts_with('/') {
                    unknown_command(model);
                }
                return true;
            }
            _ => {}
        }
    }
    if *key == Key::Enter
        && model.editor.text.starts_with('/')
        && !model.editor.text.contains('\n')
        && !model.menu.pasted_literal
    {
        let command = model.editor.text.trim();
        if let Some(entry) = menu::commands().into_iter().find(|e| e.label == command) {
            execute(model, session, entry.action);
        } else {
            unknown_command(model);
        }
        return true;
    }
    false
}
fn unknown_command(model: &mut Model) {
    let input = model.editor.take();
    let suggestion = menu::commands()
        .into_iter()
        .map(|entry| (edit_distance(&input, &entry.label), entry.label))
        .min_by_key(|(distance, _)| *distance)
        .filter(|(distance, _)| *distance <= 3);
    let note = suggestion.map_or_else(
        || "try /help".into(),
        |(_, name)| format!("did you mean {name}?"),
    );
    receipt(
        model,
        &input,
        super::lab::model::Status::Failed,
        "Unknown command",
        &note,
        Vec::new(),
    );
}
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, left) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, right) in b.iter().enumerate() {
            let substitute = diagonal + usize::from(left != *right);
            diagonal = row[j + 1];
            row[j + 1] = substitute.min(row[j] + 1).min(diagonal + 1);
        }
    }
    row[b.len()]
}

fn receipt(
    model: &mut Model,
    input: &str,
    status: super::lab::model::Status,
    result: &str,
    note: &str,
    facts: Vec<(String, String)>,
) -> usize {
    let index = model.blocks.len();
    model.blocks.push(Block {
        speaker: "CommandReceipt",
        text: format!("{input} · {result} · {note}"),
    });
    model.command_receipts.insert(
        index,
        super::lab::model::Receipt {
            input: input.into(),
            status,
            result: result.into(),
            note: note.into(),
            facts,
        },
    );
    index
}

fn execute_argument(model: &mut Model, session: &mut Session, line: &str) {
    if session.signed_out() {
        receipt(
            model,
            line,
            super::lab::model::Status::Warned,
            "Sign in required",
            "previous value kept",
            Vec::new(),
        );
        model.editor.take();
        return;
    }
    let Some(catalog) = model
        .account
        .as_ref()
        .and_then(|view| view.catalog.as_ref())
        .filter(|catalog| catalog.fresh())
        .cloned()
    else {
        if session.refresh_catalog() {
            let view = model.account.as_mut().unwrap();
            view.pending_argument = Some(line.into());
            view.loading_catalog(false);
        } else {
            receipt(
                model,
                line,
                super::lab::model::Status::Warned,
                "Catalog unavailable",
                "selection kept",
                Vec::new(),
            );
        }
        model.editor.take();
        return;
    };
    let (command, query) = line.split_once(char::is_whitespace).unwrap();
    let query = query.trim();
    let current = model.account.as_ref().unwrap().selected;
    let (selected, noun, before) = if command == "/model" {
        let choices: Vec<_> = catalog
            .entries
            .iter()
            .filter(|entry| entry.visible && entry.compatible)
            .collect();
        let labels: Vec<_> = choices
            .iter()
            .map(|entry| super::lab::picker::Choice {
                label: entry.id.clone(),
                detail: entry.name.clone(),
            })
            .collect();
        let Some(found) = super::lab::picker::matches(&labels, query).first().cloned() else {
            receipt(
                model,
                line,
                super::lab::model::Status::Warned,
                &format!("No model matches \"{query}\""),
                &format!("kept {}", current.id()),
                Vec::new(),
            );
            model.editor.take();
            return;
        };
        let id = &choices[found.index].id;
        let same_effort = session::Model::new(id, current.effort()).unwrap();
        let selected = if catalog.support(same_effort)
            == crate::providers::openai_account::catalog::Support::Unsupported
        {
            session::Model::new(id, None).unwrap()
        } else {
            same_effort
        };
        (selected, "Model", current.id())
    } else {
        let Some(entry) = catalog.entry(current.id()) else {
            receipt(
                model,
                line,
                super::lab::model::Status::Warned,
                "Current model absent from catalog",
                "effort kept",
                Vec::new(),
            );
            model.editor.take();
            return;
        };
        let mut choices = vec!["provider default".to_string()];
        choices.extend(entry.efforts.iter().flatten().cloned());
        let labels: Vec<_> = choices
            .iter()
            .map(|level| super::lab::picker::Choice {
                label: level.clone(),
                detail: String::new(),
            })
            .collect();
        let Some(found) = super::lab::picker::matches(&labels, query).first().cloned() else {
            receipt(
                model,
                line,
                super::lab::model::Status::Warned,
                &format!("No effort matches \"{query}\""),
                &format!("kept {}", current.effort().unwrap_or("provider default")),
                Vec::new(),
            );
            model.editor.take();
            return;
        };
        let effort = (found.index > 0).then(|| choices[found.index].as_str());
        (
            current.with_effort(effort).unwrap(),
            "Effort",
            current.effort().unwrap_or("provider default"),
        )
    };
    if selected == current {
        receipt(
            model,
            line,
            super::lab::model::Status::Done,
            &format!("{noun} already selected"),
            &format!("kept {before}"),
            Vec::new(),
        );
        model.editor.take();
        return;
    }
    execute(model, session, Action::Model(selected));
}

pub(super) fn complete_argument(model: &mut Model, session: &mut Session) {
    if let Some(line) = model
        .account
        .as_mut()
        .and_then(|view| view.pending_argument.take())
    {
        model.editor.replace(&line);
        if line == "/effort" {
            execute(model, session, Action::Effort);
        } else {
            execute_argument(model, session, &line);
        }
    }
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
        && matches!(
            action,
            Action::Models
                | Action::Effort
                | Action::DefaultModels
                | Action::SelectModel(..)
                | Action::Model(_)
                | Action::Context
                | Action::Compact
                | Action::DiscardPendingImages
        )
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
        Action::Clear => {
            model.navigation = Some(Request::Clear);
            true
        }
        Action::Status => {
            let view = model.account.as_ref().unwrap();
            let facts = vec![
                (
                    "session".into(),
                    view.id.clone().unwrap_or_else(|| "pending".into()),
                ),
                ("model".into(), view.selected.id().into()),
                (
                    "effort".into(),
                    view.selected.effort().unwrap_or("provider default").into(),
                ),
                (
                    "directory".into(),
                    view.directory
                        .clone()
                        .unwrap_or_else(|| "Conversation only".into()),
                ),
                (
                    "access".into(),
                    if view.file_tools {
                        view.access.name()
                    } else {
                        "no file tools"
                    }
                    .into(),
                ),
            ];
            receipt(
                model,
                "/status",
                super::lab::model::Status::Done,
                "Session",
                "",
                facts,
            );
            model.menu.close();
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
            if let Some(catalog) = model
                .account
                .as_ref()
                .unwrap()
                .catalog
                .as_ref()
                .filter(|catalog| catalog.fresh())
            {
                let selected = model.account.as_ref().unwrap().selected;
                model.menu.open(menu::models(catalog, selected, false));
                true
            } else if session.refresh_catalog() {
                model.account.as_mut().unwrap().loading_catalog(false);
                model.menu.close();
                true
            } else {
                false
            }
        }
        Action::Effort => {
            if let Some(catalog) = model
                .account
                .as_ref()
                .unwrap()
                .catalog
                .as_ref()
                .filter(|catalog| catalog.fresh())
            {
                let selected = model.account.as_ref().unwrap().selected;
                if let Some(entry) = catalog.entry(selected.id()) {
                    model.menu.open(menu::efforts(entry, selected, false));
                    true
                } else {
                    receipt(
                        model,
                        "/effort",
                        super::lab::model::Status::Warned,
                        "Current model absent from catalog",
                        "effort kept",
                        Vec::new(),
                    );
                    true
                }
            } else if session.refresh_catalog() {
                let view = model.account.as_mut().unwrap();
                view.pending_argument = Some("/effort".into());
                view.loading_catalog(false);
                model.menu.close();
                true
            } else {
                false
            }
        }
        Action::DefaultModels => {
            if let Some(catalog) = model
                .account
                .as_ref()
                .unwrap()
                .catalog
                .as_ref()
                .filter(|catalog| catalog.fresh())
            {
                match Settings::user() {
                    Ok(settings) => {
                        model.menu.open(menu::models(catalog, settings.model, true));
                        true
                    }
                    Err(_) => {
                        notice(model, "Cannot read saved defaults / file kept");
                        false
                    }
                }
            } else if session.refresh_catalog() {
                model.account.as_mut().unwrap().loading_catalog(true);
                model.menu.close();
                true
            } else {
                false
            }
        }
        Action::SelectModel(id, defaults) => {
            let entry = model
                .account
                .as_ref()
                .unwrap()
                .catalog
                .as_ref()
                .and_then(|catalog| catalog.entry(&id))
                .cloned();
            match entry {
                Some(entry) if entry.visible && entry.compatible => {
                    let current = if defaults {
                        Settings::user()
                            .map(|settings| settings.model)
                            .unwrap_or(model.account.as_ref().unwrap().selected)
                    } else {
                        model.account.as_ref().unwrap().selected
                    };
                    model.menu.open(menu::efforts(&entry, current, defaults));
                    true
                }
                _ => {
                    notice(model, "Model no longer available / selection kept");
                    false
                }
            }
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
            let previous = model.account.as_ref().unwrap().selected;
            let typed = model.editor.text.trim();
            let line = if typed.starts_with("/model ") || typed.starts_with("/effort ") {
                typed.to_owned()
            } else if selected.id() != previous.id() {
                format!("/model {}", selected.id())
            } else {
                format!(
                    "/effort {}",
                    selected.effort().unwrap_or("provider default")
                )
            };
            if !session.set_model(selected) {
                receipt(
                    model,
                    &line,
                    super::lab::model::Status::Warned,
                    "Selection unavailable",
                    "previous value kept",
                    Vec::new(),
                );
                return;
            }
            let index = receipt(
                model,
                &line,
                super::lab::model::Status::Running,
                "Applying selection",
                "waiting for saved acknowledgement",
                Vec::new(),
            );
            model.account.as_mut().unwrap().pending_model_receipt = Some((index, previous));
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
                if session.selection_unavailable() {
                    notice(model, super::account::UNAVAILABLE_SELECTION_NOTICE);
                    model.account.as_mut().unwrap().local_failed = true;
                }
                return;
            }
            model.menu.close();
            super::account::compacting(model);
            true
        }
        Action::DiscardPendingImages => {
            if !session.discard_pending_images() {
                return;
            }
            model.menu.close();
            super::account::discarding_pending_images(model);
            true
        }
        Action::Help => {
            let facts = menu::commands()
                .into_iter()
                .map(|entry| (entry.label, entry.description))
                .collect();
            receipt(
                model,
                "/help",
                super::lab::model::Status::Done,
                "Commands",
                "Enter queues a new turn while busy",
                facts,
            );
            model.menu.close();
            true
        }
    };
    if done {
        model.editor.take();
        model.menu.pasted_literal = false;
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
    fn local_help_describes_the_same_direct_execution_as_cli() {
        let mut session = session::tests::ready_fixture();
        let mut model = account::model(session::Model::Luna, None);
        account::event(&mut model, Event::Ready);
        execute(&mut model, &mut session, Action::Help);
        let help = model.command_receipts.values().last().unwrap();
        assert_eq!(help.input, "/help");
        assert!(help.facts.iter().any(|(name, _)| name == "/clear"));
        assert!(help.facts.iter().any(|(name, _)| name == "/effort"));
    }

    #[test]
    fn rejected_compaction_keeps_draft_and_shows_local_model_notice() {
        let catalog = crate::providers::openai_account::catalog::Catalog::parse(br#"{"models":[{"slug":"replacement","visibility":"list","supported_reasoning_levels":[{"effort":"high"}]}]}"#).unwrap();
        let mut session = session::tests::ready_fixture_with_catalog(catalog.clone());
        let mut model = account::model(session::Model::Luna, None);
        account::event(&mut model, Event::CatalogLoaded(catalog));
        account::event(&mut model, Event::Ready);
        model.editor.insert("unsent draft");
        execute(&mut model, &mut session, Action::Compact);
        let notice = &model.account.as_ref().unwrap().local_notice;
        assert!(notice.contains("/model"), "{notice}");
        assert!(model.account.as_ref().unwrap().local_failed);
        assert_eq!(model.editor.text, "unsent draft");
        assert!(model.blocks.is_empty());
        assert!(session.ready());
        assert!(session.poll().is_none());
        let rows = view::chrome(&model, 80, 24);
        let rules: Vec<_> = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.text.starts_with('─'))
            .map(|(index, _)| index)
            .collect();
        let notice_row = rows
            .iter()
            .position(|row| row.text.contains("/model"))
            .unwrap();
        assert!(rules[0] < notice_row && notice_row < rules[1]);
    }

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
