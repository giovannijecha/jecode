use super::*;
use std::thread;
use std::time::{Duration, Instant};

fn wait_checkpoint(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.checkpoint_job.is_some() {
        app.poll().unwrap();
        assert!(Instant::now() < deadline, "checkpoint list did not finish");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_checkpoint_list_returns_to_commands_after_its_worker_finishes() {
    let (home, project) = (Directory::new(), Directory::new());
    let mut app = drafted(&home, &project);
    app.agent.as_mut().unwrap().enable_checkpoints(home.path());
    let before = app.state.editor.clone();
    key(&mut app, CTRL_G, CTRL);
    text(&mut app, "checkpoints");
    key(&mut app, ENTER, 0);
    assert!(app.checkpoint_job.is_some());
    wait_checkpoint(&mut app);
    assert!(app.state.information.is_some());
    key(&mut app, ESC, 0);
    assert!(commands_open(&app));
    assert_eq!(app.state.editor, before);
}

#[test]
fn a_checkpoint_completion_does_not_open_under_a_new_question() {
    let (home, project) = (Directory::new(), Directory::new());
    let mut app = drafted(&home, &project);
    app.agent.as_mut().unwrap().enable_checkpoints(home.path());
    let request = question();
    assert!(app.question_observer.begin(request).unwrap());
    app.refresh_question();
    key(&mut app, ESC, 0);
    key(&mut app, CTRL_G, CTRL);
    text(&mut app, "checkpoints");
    key(&mut app, ENTER, 0);
    assert!(app.checkpoint_job.is_some());
    key(&mut app, CTRL_G, CTRL);
    text(&mut app, "/ask");
    key(&mut app, ENTER, 0);
    assert!(app.state.ask_panel.is_some());
    wait_checkpoint(&mut app);
    assert!(app.state.ask_panel.is_some());
    assert!(app.state.information.is_none());
    key(&mut app, ESC, 0);
    assert!(commands_open(&app));
}

#[test]
fn a_withdrawn_question_leaves_esc_with_the_composer_after_one_poll() {
    let (home, project) = (Directory::new(), Directory::new());
    let mut app = drafted(&home, &project);
    let before = app.state.editor.clone();
    let request = question();
    assert!(app.question_observer.begin(request.clone()).unwrap());
    app.refresh_question();
    key(&mut app, ESC, 0);
    key(&mut app, CTRL_G, CTRL);
    text(&mut app, "/ask");
    key(&mut app, ENTER, 0);
    assert!(app.state.ask_panel.is_some() && app.from_commands);
    app.question_observer.complete(&request.id);
    app.poll().unwrap();
    assert!(app.state.ask_panel.is_none());
    key(&mut app, ESC, 0);
    assert!(!commands_open(&app));
    assert_eq!(app.state.editor, before);
}
