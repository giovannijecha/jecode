use super::*;
use crate::{json::Value, openrouter::OpenRouter, test_support::Directory, tools::Tools};
use selector::Purpose;

fn app(home: &Directory, directory: &Directory) -> App {
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    App::new(agent, tests::config(directory), None)
}
fn seed(app: &mut App, title: &str) {
    for (role, text) in [("user", title), ("assistant", "Retained answer")] {
        app.archive.messages.lock().unwrap().push(Value::object([
            ("role", Value::string(role)),
            ("content", Value::string(text)),
        ]));
        app.state.message(
            if role == "user" {
                Kind::User
            } else {
                Kind::Assistant
            },
            text,
        );
    }
    app.agent.as_ref().unwrap().save_session().unwrap();
}
fn key(app: &mut App, code: u16, modifiers: u8) -> bool {
    app.input(Decoded::Key(terminal::Key {
        code,
        modifiers,
        character: 0,
    }))
    .unwrap()
}
fn select(app: &mut App, id: &str) {
    let index = app
        .saved_sessions
        .iter()
        .position(|entry| entry.id == id)
        .unwrap();
    let menu = app.state.selector.as_mut().unwrap();
    menu.selected = menu
        .filtered
        .iter()
        .position(|hit| hit.index == index)
        .unwrap();
}
fn confirm(app: &mut App, id: &str) {
    if app.state.selector.is_none() {
        app.dispatch("/resume".into()).unwrap();
    }
    select(app, id);
    assert!(!key(app, 68, 4));
    assert!(
        app.state
            .selector
            .as_ref()
            .unwrap()
            .delete_target()
            .is_some()
    );
    assert!(app.deletion_job.is_none());
    assert!(!key(app, 13, 0));
    assert!(app.deletion_job.is_some());
}

#[test]
fn opening_drafting_and_local_commands_do_not_save_a_conversation() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut current = app(&home, &directory);
    let store = current.persistence.as_ref().unwrap().store();
    current.state.editor.insert("unsent draft 🦀");
    current.persist_input(true);
    current.dispatch("/help".into()).unwrap();
    current.dispatch("/new".into()).unwrap();
    current.persist_input(true);
    assert!(store.list().unwrap().sessions.is_empty());
    assert!(!home.path().join("sessions").exists());
    seed(&mut current, "First sent message");
    assert_eq!(store.list().unwrap().sessions.len(), 1);
    drop(current);
    assert_eq!(store.list().unwrap().sessions.len(), 1);
}

#[test]
fn delete_current_resets_screen_and_context_preserves_unsent_work_and_cannot_resurrect() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut current = app(&home, &directory);
    seed(&mut current, "Delete this conversation");
    let old = current.persistence.as_ref().unwrap().clone();
    let id = old.id();
    let store = old.store();
    current.state.editor.insert("kept draft 🦀");
    current.state.editor.cursor = 5;
    current.state.queue.push("follow-up kept".into()).unwrap();
    current.state.history.record("Delete this conversation");
    current
        .state
        .history
        .navigate(&mut current.state.editor, true);
    assert_eq!(current.state.editor.text, "Delete this conversation");
    confirm(&mut current, &id);
    current.finish_delete(true).unwrap();
    assert_ne!(current.persistence.as_ref().unwrap().id(), id);
    assert!(store.list().unwrap().sessions.is_empty());
    assert_eq!(current.archive.messages.lock().unwrap().len(), 1);
    assert!(current.state.items.is_empty());
    assert_eq!(current.state.editor.text, "kept draft 🦀");
    assert_eq!(current.state.editor.cursor, 5);
    assert_eq!(
        current.state.queue.messages.front().map(String::as_str),
        Some("follow-up kept")
    );
    assert!(current.state.history.snapshot().is_empty());
    old.input(crate::sessions::Input::default());
    old.flush().unwrap();
    assert!(store.list().unwrap().sessions.is_empty());
    let frame =
        current
            .renderer
            .update(&current.state, "fixture/default", "fixture", Instant::now());
    assert!(frame.contains("\x1b[2J") && !frame.contains("\x1b[3J"));
    assert!(!frame.contains("Delete this conversation"));
    current.renderer.resized((44, 12), Instant::now());
    current.state.width = 44;
    current.state.height = 12;
    let frame = current.renderer.update(
        &current.state,
        "fixture/default",
        "fixture",
        Instant::now() + Duration::from_millis(75),
    );
    assert!(!frame.contains("Delete this conversation"));
}

#[test]
fn escape_disarms_deletion_in_the_same_list_and_then_closes_without_changing_the_draft() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut current = app(&home, &directory);
    seed(&mut current, "Keep this conversation");
    current.state.editor.insert("unsent");
    let id = current.persistence.as_ref().unwrap().id();
    current.dispatch("/resume".into()).unwrap();
    assert!(matches!(
        current.state.selector.as_ref().unwrap().purpose,
        Purpose::Sessions { .. }
    ));
    select(&mut current, &id);
    key(&mut current, 68, 4);
    key(&mut current, 27, 0);
    assert!(
        current
            .state
            .selector
            .as_ref()
            .unwrap()
            .delete_target()
            .is_none()
    );
    assert!(current.deletion_job.is_none());
    key(&mut current, 27, 0);
    assert!(current.state.selector.is_none());
    assert_eq!(current.state.editor.text, "unsent");
    assert_eq!(current.persistence.as_ref().unwrap().id(), id);
    assert_eq!(
        current
            .persistence
            .as_ref()
            .unwrap()
            .store()
            .list()
            .unwrap()
            .sessions
            .len(),
        1
    );
}

#[test]
fn delete_other_session_and_busy_errors_leave_current_context_intact() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut first = app(&home, &directory);
    seed(&mut first, "Old conversation");
    let old_id = first.persistence.as_ref().unwrap().id();
    let mut current = app(&home, &directory);
    seed(&mut current, "Current conversation");
    let id = current.persistence.as_ref().unwrap().id();
    let before = current.archive.messages.lock().unwrap().clone();
    confirm(&mut current, &old_id);
    current.finish_delete(true).unwrap();
    assert!(matches!(
        current.state.notice,
        Some(Feedback {
            kind: Kind::Error,
            ..
        })
    ));
    assert!(
        current
            .state
            .selector
            .as_ref()
            .unwrap()
            .delete_target()
            .is_none()
    );
    assert_eq!(*current.archive.messages.lock().unwrap(), before);
    drop(first);
    confirm(&mut current, &old_id);
    current.finish_delete(true).unwrap();
    assert!(current.state.selector.is_some());
    assert!(
        current
            .saved_sessions
            .iter()
            .all(|entry| entry.id != old_id)
    );
    assert_eq!(current.persistence.as_ref().unwrap().id(), id);
    assert_eq!(*current.archive.messages.lock().unwrap(), before);
    assert_eq!(
        current
            .persistence
            .as_ref()
            .unwrap()
            .store()
            .list()
            .unwrap()
            .sessions
            .len(),
        1
    );
}

#[test]
fn empty_current_is_not_selectable_and_removed_command_is_local() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut current = app(&home, &directory);
    let id = current.persistence.as_ref().unwrap().id();
    current.open_sessions();
    assert!(current.saved_sessions.is_empty());
    key(&mut current, 68, 4);
    key(&mut current, 13, 0);
    assert!(current.deletion_job.is_none());
    let fresh = current.persistence.as_ref().unwrap().id();
    assert_eq!(id, fresh);
    assert!(
        current
            .persistence
            .as_ref()
            .unwrap()
            .store()
            .list()
            .unwrap()
            .sessions
            .is_empty()
    );
    current.dispatch("/delete ../config".into()).unwrap();
    assert_eq!(current.persistence.as_ref().unwrap().id(), fresh);
    assert!(
        current
            .state
            .items
            .iter()
            .any(|item| matches!(item, state::Item::Local {
        kind: Kind::Error, result: Some(result), ..
    } if result.contains("Unknown command")))
    );
}

#[test]
fn deletion_of_a_filtered_row_keeps_search_and_supports_repeated_deletion() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut ids = vec![];
    for index in 0..9 {
        let mut saved = app(&home, &directory);
        seed(&mut saved, &format!("Saved request {index}"));
        ids.push(saved.persistence.as_ref().unwrap().id());
    }
    let mut current = app(&home, &directory);
    let current_id = current.persistence.as_ref().unwrap().id();
    current.state.editor.insert("kept draft");
    current.dispatch("/resume".into()).unwrap();
    current.input(Decoded::Text("request 4".into())).unwrap();
    let menu = current.state.selector.as_ref().unwrap();
    assert_eq!(menu.filtered.len(), 1);
    assert_ne!(menu.filtered[0].index, 0);
    confirm(&mut current, &ids[4]);
    // A confirmed operation is frozen until its worker returns.
    current.input(Decoded::Text("1".into())).unwrap();
    key(&mut current, 40, 0);
    key(&mut current, 27, 0);
    assert_eq!(
        current.state.selector.as_ref().unwrap().editor.text,
        "request 4"
    );
    current.finish_delete(true).unwrap();
    let menu = current.state.selector.as_mut().unwrap();
    assert_eq!(menu.editor.text, "request 4");
    assert!(menu.filtered.is_empty());
    assert!(menu.delete_target().is_none());
    key(&mut current, 68, 4);
    assert!(current.deletion_job.is_none());
    current
        .state
        .selector
        .as_mut()
        .unwrap()
        .editor
        .replace("request 5".into());
    current.state.selector.as_mut().unwrap().refresh();
    confirm(&mut current, &ids[5]);
    current.finish_delete(true).unwrap();
    assert_eq!(current.persistence.as_ref().unwrap().id(), current_id);
    assert_eq!(current.state.editor.text, "kept draft");
    assert!(current.state.selector.as_ref().unwrap().searchable);
    let saved = current
        .persistence
        .as_ref()
        .unwrap()
        .store()
        .list()
        .unwrap();
    assert_eq!(saved.sessions.len(), 7);
    assert!(
        saved
            .sessions
            .iter()
            .all(|entry| entry.id != ids[4] && entry.id != ids[5])
    );
}

#[test]
fn current_row_enter_is_a_noop_and_confirmation_cannot_follow_navigation_or_a_number() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut saved = app(&home, &directory);
    seed(&mut saved, "Saved request");
    drop(saved);
    let mut current = app(&home, &directory);
    seed(&mut current, "Current request");
    let id = current.persistence.as_ref().unwrap().id();
    current.state.editor.insert("kept draft");
    current.dispatch("/resume".into()).unwrap();
    select(&mut current, &id);
    key(&mut current, 13, 0);
    assert!(current.state.selector.is_none());
    assert_eq!(current.persistence.as_ref().unwrap().id(), id);
    current.open_sessions();
    select(&mut current, &id);
    key(&mut current, 68, 4);
    key(&mut current, 13, 4);
    assert!(current.deletion_job.is_none());
    key(&mut current, 68, 4);
    assert!(
        current
            .state
            .selector
            .as_ref()
            .unwrap()
            .delete_target()
            .is_none()
    );
    key(&mut current, 68, 4);
    current.input(Decoded::Text("2".into())).unwrap();
    assert!(current.deletion_job.is_none());
    assert!(
        current
            .state
            .selector
            .as_ref()
            .unwrap()
            .delete_target()
            .is_none()
    );
    key(&mut current, 68, 4);
    key(&mut current, 40, 0);
    assert!(
        current
            .state
            .selector
            .as_ref()
            .unwrap()
            .delete_target()
            .is_none()
    );
    assert_eq!(current.state.editor.text, "kept draft");
    assert_eq!(current.persistence.as_ref().unwrap().id(), id);
}

#[test]
fn editing_search_cancels_the_delete_mark_and_quitting_an_armed_list_keeps_sessions() {
    let home = Directory::new();
    let directory = Directory::new();
    for index in 0..9 {
        let mut saved = app(&home, &directory);
        seed(&mut saved, &format!("Saved request {index}"));
    }
    let mut current = app(&home, &directory);
    current.open_sessions();
    current
        .input(Decoded::Text("Saved request".into()))
        .unwrap();
    key(&mut current, 68, 4);
    current.input(Decoded::Text(" 5".into())).unwrap();
    assert!(
        current
            .state
            .selector
            .as_ref()
            .unwrap()
            .delete_target()
            .is_none()
    );
    key(&mut current, 68, 4);
    assert!(key(&mut current, 81, 4));
    assert!(current.deletion_job.is_none());
    drop(current);
    let next = app(&home, &directory);
    assert_eq!(
        next.persistence
            .as_ref()
            .unwrap()
            .store()
            .list()
            .unwrap()
            .sessions
            .len(),
        9
    );
}

#[test]
fn deleting_another_session_saves_its_result_before_exit_without_another_command() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut saved = app(&home, &directory);
    seed(&mut saved, "Delete this saved request");
    let deleted = saved.persistence.as_ref().unwrap().id();
    drop(saved);
    let mut current = app(&home, &directory);
    seed(&mut current, "Keep current request");
    let id = current.persistence.as_ref().unwrap().id();
    confirm(&mut current, &deleted);
    current.finish_delete(true).unwrap();
    drop(current);
    let mut restored = app(&home, &directory);
    restored.resume_session(&id);
    assert!(
        restored
            .persistence
            .as_ref()
            .unwrap()
            .snapshot()
            .events
            .iter()
            .any(|event| event.get("command").and_then(Value::as_str)
                == Some(&format!("/resume · delete {deleted}")))
    );
}
