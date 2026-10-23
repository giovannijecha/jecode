use super::state::Item;
use super::*;
use crate::{
    backend::openrouter::OpenRouter, checkpoints::session::Session, test_support::Directory,
    tools::Tools,
};
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

fn submit(app: &mut App, text: &str) {
    app.state.editor.replace(text.into());
    app.edited();
    assert!(!app.submit().unwrap());
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
fn checkpoint_list_and_show_run_off_the_input_thread() {
    let home = Directory::new();
    let project = Directory::new();
    fs::write(project.path().join("a.txt"), "saved before\n").unwrap();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:9".into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let session = Session::offline(home.path(), project.path());
    session.begin("fixture");
    let reference = session
        .store()
        .unwrap()
        .capture(
            "write",
            "fixture",
            &["a.txt".into()],
            &crate::cancel::Cancellation::default(),
        )
        .unwrap()
        .reference
        .unwrap();
    fs::write(project.path().join("a.txt"), "after").unwrap();
    agent.use_checkpoints(session);
    let mut app = App::new(agent, tests::config(&project), None);

    submit(&mut app, "/checkpoints list");
    assert!(app.checkpoint_job.is_some());
    wait_checkpoint(&mut app);
    assert!(app.state.items.iter().any(|item| matches!(
        item,
        Item::Local { result: Some(result), details, .. }
            if result.contains("Checkpoint") && details.iter().any(|(label, value)| label.contains(&reference) || value.contains(&reference))
    )));

    app.state.information = None;
    submit(&mut app, &format!("/checkpoints show {reference} a.txt"));
    wait_checkpoint(&mut app);
    let information = app
        .state
        .information
        .as_ref()
        .expect("show opens information panel");
    assert!(
        information
            .details
            .iter()
            .any(|(_, value)| value.contains("saved before"))
    );
}
