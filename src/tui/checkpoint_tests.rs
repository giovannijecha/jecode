use super::state::Item;
use super::*;
use crate::{
    checkpoints::session::Session, openrouter::OpenRouter, test_support::Directory, tools::Tools,
};
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

fn submit(app: &mut App, text: &str) {
    app.state.editor.replace(text.into());
    app.edited();
    assert!(!app.submit().unwrap());
}

/// A local command, its result and its details.
type Local = (String, Option<String>, Vec<(String, String)>);

fn locals(app: &App) -> Vec<Local> {
    app.state
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Local {
                command,
                result,
                details,
                ..
            } => Some((command.clone(), result.clone(), details.clone())),
            _ => None,
        })
        .collect()
}

fn wait_checkpoint(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.checkpoint_job.is_some() {
        app.poll().unwrap();
        assert!(
            Instant::now() < deadline,
            "checkpoint command did not finish"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn checkpoint_commands_open_information_and_never_copy_recorded_files_into_the_conversation() {
    let home = Directory::new();
    let project = Directory::new();
    fs::write(project.path().join("a.txt"), "secret recorded line\n").unwrap();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:9".into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent.use_checkpoints(Session::offline(home.path(), project.path()));
    let mut app = App::new(agent, tests::config(&project), None);

    submit(&mut app, "/checkpoints");
    assert!(
        app.checkpoint_job.is_some(),
        "the command must leave the input thread"
    );
    wait_checkpoint(&mut app);
    let information = app
        .state
        .information
        .as_ref()
        .expect("status opens the information panel");
    assert!(
        information.title.starts_with("Checkpoints"),
        "{}",
        information.title
    );
    let latest = information
        .details
        .iter()
        .find(|(key, _)| key == "latest")
        .unwrap()
        .1
        .clone();

    app.state.information = None;
    submit(&mut app, &format!("/checkpoints show {latest} a.txt"));
    wait_checkpoint(&mut app);
    let information = app
        .state
        .information
        .as_ref()
        .unwrap_or_else(|| panic!("{:?}", locals(&app)));
    assert!(
        information
            .details
            .iter()
            .any(|(_, text)| text.contains("secret recorded line"))
    );
    let recorded = locals(&app);
    let (command, result, details) = recorded.last().unwrap();
    assert_eq!(command, &format!("/checkpoints show {latest} a.txt"));
    assert!(
        result
            .as_deref()
            .is_some_and(|result| !result.contains("secret"))
    );
    assert!(details.is_empty(), "{details:?}");

    app.state.information = None;
    submit(&mut app, "/checkpoints explode");
    wait_checkpoint(&mut app);
    let recorded = locals(&app);
    assert!(
        recorded
            .last()
            .unwrap()
            .1
            .as_deref()
            .is_some_and(|result| result.contains("explode"))
    );
}

#[test]
fn a_checkpoint_command_queues_session_switches_until_its_result_is_shown() {
    let home = Directory::new();
    let project = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:9".into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent.use_checkpoints(Session::offline(home.path(), project.path()));
    let mut app = App::new(agent, tests::config(&project), None);
    let original = app.persistence.as_ref().unwrap().id();

    submit(&mut app, "/checkpoints verify");
    assert!(app.checkpoint_job.is_some());
    app.input(Decoded::Text("still editable".into())).unwrap();
    assert_eq!(app.state.editor.text, "still editable");
    app.state.editor.replace(String::new());
    app.input(Decoded::Key(terminal::Key {
        code: 112,
        modifiers: 0,
        character: 0,
    }))
    .unwrap();
    assert_eq!(locals(&app).last().unwrap().0, "/checkpoints verify");
    submit(&mut app, "/new");
    assert_eq!(app.persistence.as_ref().unwrap().id(), original);
    assert_eq!(app.state.queue.messages().len(), 1);

    wait_checkpoint(&mut app);
    assert_eq!(app.persistence.as_ref().unwrap().id(), original);
    assert!(app.state.information.is_some());
    app.state.information = None;
    app.poll().unwrap();
    assert_ne!(app.persistence.as_ref().unwrap().id(), original);
    assert!(app.state.queue.messages().is_empty());
}
