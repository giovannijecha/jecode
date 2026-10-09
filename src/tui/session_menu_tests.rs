use super::*;
use crate::{json::Value, openrouter::OpenRouter, test_support::Directory, tools::Tools};

fn app(home: &Directory, project: &Directory) -> App {
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    App::new(agent, tests::config(project), None)
}

fn seed(app: &App, title: &str) {
    app.archive.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("user")),
        ("content", Value::string(title)),
    ]));
    app.agent.as_ref().unwrap().save_session().unwrap();
}

fn delete(app: &mut App, id: &str) {
    let index = app
        .saved_sessions
        .iter()
        .position(|session| session.id == id)
        .unwrap();
    let menu = app.state.selector.as_mut().unwrap();
    menu.selected = menu
        .filtered
        .iter()
        .position(|hit| hit.index == index)
        .unwrap();
    for (code, character, modifiers) in [(68, 4, 4), (13, 13, 0)] {
        app.input(Decoded::Key(terminal::Key {
            code,
            character,
            modifiers,
        }))
        .unwrap();
    }
    app.finish_delete(true).unwrap();
}

#[test]
fn a_fresh_launch_and_unsent_work_never_count_as_a_session_in_resume() {
    let home = Directory::new();
    let project = Directory::new();
    let mut current = app(&home, &project);
    current.state.editor.insert("unsent draft");
    current.state.queue.push("unsent follow-up").unwrap();
    current.persist_input(true);
    current.dispatch("/help".into()).unwrap();
    current.dispatch("/resume".into()).unwrap();
    assert!(current.saved_sessions.is_empty());
    assert!(current.state.selector.as_ref().unwrap().options.is_empty());
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
    seed(&current, "First sent request");
    current.open_sessions();
    assert_eq!(current.saved_sessions.len(), 1);
    assert_eq!(
        current.saved_sessions[0].id,
        current.persistence.as_ref().unwrap().id()
    );
}

#[test]
fn current_and_last_saved_deletions_keep_resume_open_without_an_empty_replacement_row() {
    let home = Directory::new();
    let project = Directory::new();
    let saved = app(&home, &project);
    seed(&saved, "Other saved conversation");
    let saved_id = saved.persistence.as_ref().unwrap().id();
    drop(saved);
    let mut current = app(&home, &project);
    seed(&current, "Current saved conversation");
    let current_id = current.persistence.as_ref().unwrap().id();
    current.state.editor.insert("kept draft");
    current.state.queue.push("kept follow-up").unwrap();
    current.open_sessions();
    delete(&mut current, &current_id);
    assert!(current.state.selector.is_some());
    assert_eq!(current.saved_sessions.len(), 1);
    assert_eq!(current.saved_sessions[0].id, saved_id);
    assert_ne!(current.persistence.as_ref().unwrap().id(), current_id);
    assert_eq!(current.state.editor.text, "kept draft");
    delete(&mut current, &saved_id);
    assert!(current.state.selector.as_ref().unwrap().options.is_empty());
    assert!(current.saved_sessions.is_empty());
    current.poll().unwrap();
    assert!(current.worker.is_none());
    assert_eq!(
        current
            .state
            .queue
            .messages
            .front()
            .map(|prompt| prompt.text.as_str()),
        Some("kept follow-up")
    );
    let visible = view::frame(&current.state, "fixture/model", "fixture")
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(visible.contains("No saved conversations"));
    assert!(visible.contains("Esc close"));
}
