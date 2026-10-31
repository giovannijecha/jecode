use super::tests::{rows, wait_until};
use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, Response, completion},
    tools::Tools,
};
use std::{sync::mpsc, thread, time::Instant};

const STOP: &str =
    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";

fn gated(release: mpsc::Receiver<()>) -> Response {
    Response::GatedStream {
        head:
            "data: {\"choices\":[{\"delta\":{\"content\":\"Active.\"},\"finish_reason\":null}]}\n\n"
                .into(),
        tail: STOP.into(),
        release,
    }
}

fn app(home: &Directory, directory: &Directory, fixture: &HttpFixture) -> App {
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    App::new(agent, tests::config(directory), None)
}

fn key(app: &mut App, code: u16, modifiers: u8) {
    app.input(Decoded::Key(terminal::Key {
        code,
        modifiers,
        character: 0,
    }))
    .unwrap();
}

fn submit(app: &mut App, value: &str) {
    app.input(Decoded::Text(value.into())).unwrap();
    key(app, 13, 0);
}

/// Starts a turn and waits until its first response is streaming.
fn start(app: &mut App, prompt: &str) {
    submit(app, prompt);
    wait_until(app, |app| {
        app.state
            .items
            .iter()
            .any(|item| matches!(item, state::Item::Streaming { .. }))
    });
}

fn worker_finished(app: &App) {
    let started = Instant::now();
    while !app.worker.as_ref().unwrap().finished() {
        assert!(started.elapsed() < Duration::from_secs(6));
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn an_open_panel_does_not_defer_delivery_and_no_completion_shows_between_responses() {
    let home = Directory::new();
    let directory = Directory::new();
    let (release, gate) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![
        gated(gate),
        Response::Json(200, completion("Done.", vec![])),
    ]);
    let mut app = app(&home, &directory, &fixture);
    start(&mut app, "active");
    submit(&mut app, "joined");
    key(&mut app, 112, 0);
    assert!(app.state.information.is_some());
    release.send(()).unwrap();
    wait_until(&mut app, |app| {
        rows(app).contains(&"user: joined".to_owned())
    });
    // The same turn continues: no completion notice while it works.
    assert!(app.worker.is_some());
    assert!(app.state.activity.is_some());
    assert_ne!(app.state.status, "Ready");
    assert!(app.state.queue.is_empty());
    assert!(app.state.information.is_some());
    wait_until(&mut app, |app| app.worker.is_none());
    assert_eq!(
        rows(&app),
        [
            "user: active",
            "assistant: Active.",
            "user: joined",
            "assistant: Done."
        ]
    );
    assert_eq!(app.state.history.snapshot(), ["active", "joined"]);
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn an_accepted_message_whose_notification_was_lost_is_shown_once_and_not_paused() {
    let home = Directory::new();
    let directory = Directory::new();
    let (release, gate) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![
        gated(gate),
        Response::Json(200, completion("Done.", vec![])),
    ]);
    let mut app = app(&home, &directory, &fixture);
    start(&mut app, "active");
    submit(&mut app, "joined");
    release.send(()).unwrap();
    worker_finished(&app);
    // Drop every notification, including the acceptance.
    while app.worker.as_ref().unwrap().events.try_recv().is_ok() {}
    app.poll().unwrap();
    assert!(app.worker.is_none());
    assert_eq!(
        rows(&app)
            .iter()
            .filter(|row| *row == "user: joined")
            .count(),
        1
    );
    assert!(app.state.queue.is_empty());
    assert_eq!(app.state.history.snapshot(), ["active", "joined"]);
    let input = app.persistence.as_ref().unwrap().snapshot().input;
    assert!(input.queued.is_empty() && input.paused.is_none());
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn quitting_before_polling_acceptance_saves_the_sent_prompt_in_history() {
    let home = Directory::new();
    let directory = Directory::new();
    let (release, gate) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![
        gated(gate),
        Response::Json(200, completion("Done.", vec![])),
    ]);
    let mut app = app(&home, &directory, &fixture);
    start(&mut app, "active");
    submit(&mut app, "joined");
    release.send(()).unwrap();
    worker_finished(&app);
    let handle = app.persistence.clone().unwrap();
    // The acceptance event is still buffered when Ctrl+Q drops the app.
    drop(app);
    let input = handle.store().fixture_load(&handle.id()).unwrap().input;
    assert_eq!(input.history, ["active", "joined"]);
    assert!(input.queued.is_empty());
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn quitting_after_acceptance_notification_is_lost_saves_prompt_history() {
    let home = Directory::new();
    let directory = Directory::new();
    let (release, gate) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![
        gated(gate),
        Response::Json(200, completion("Done.", vec![])),
    ]);
    let mut app = app(&home, &directory, &fixture);
    start(&mut app, "active");
    submit(&mut app, "joined");
    release.send(()).unwrap();
    worker_finished(&app);
    while app.worker.as_ref().unwrap().events.try_recv().is_ok() {}
    let handle = app.persistence.clone().unwrap();
    drop(app);
    let input = handle.store().fixture_load(&handle.id()).unwrap().input;
    assert_eq!(input.history, ["active", "joined"]);
    assert!(input.queued.is_empty());
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn quitting_after_acceptance_was_displayed_does_not_duplicate_prompt_history() {
    let home = Directory::new();
    let directory = Directory::new();
    let (release, gate) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![
        gated(gate),
        Response::Json(200, completion("Done.", vec![])),
    ]);
    let mut app = app(&home, &directory, &fixture);
    start(&mut app, "active");
    submit(&mut app, "joined");
    release.send(()).unwrap();
    wait_until(&mut app, |app| {
        rows(app).contains(&"user: joined".to_owned())
    });
    worker_finished(&app);
    let handle = app.persistence.clone().unwrap();
    drop(app);
    let input = handle.store().fixture_load(&handle.id()).unwrap().input;
    assert_eq!(input.history, ["active", "joined"]);
    assert!(input.queued.is_empty());
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn up_in_an_empty_composer_takes_the_draft_back_and_enter_queues_it_again() {
    let home = Directory::new();
    let directory = Directory::new();
    let (release, gate) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![
        gated(gate),
        Response::Json(200, completion("Done.", vec![])),
    ]);
    let mut app = app(&home, &directory, &fixture);
    start(&mut app, "active");
    submit(&mut app, "queued");
    assert_eq!(app.state.queue.pending(), Some(("queued".into(), false)));
    // A second message waits in the composer.
    submit(&mut app, "second");
    assert_eq!(app.state.editor.text, "second");
    assert_eq!(app.state.queue.messages(), ["queued"]);
    assert_eq!(
        app.state.notice.as_ref().unwrap().text,
        crate::tui::drafts::PENDING
    );
    // With text in the composer, Up only edits it.
    key(&mut app, 38, 0);
    assert_eq!(app.state.editor.text, "second");
    assert_eq!(app.state.queue.messages(), ["queued"]);
    app.state.editor.take();
    key(&mut app, 38, 0);
    assert_eq!(app.state.editor.text, "queued");
    assert!(app.state.queue.is_empty());
    submit(&mut app, " edited");
    assert!(app.state.editor.text.is_empty());
    assert_eq!(app.state.queue.messages(), ["queued edited"]);
    release.send(()).unwrap();
    wait_until(&mut app, |app| app.worker.is_none());
    assert_eq!(
        rows(&app),
        [
            "user: active",
            "assistant: Active.",
            "user: queued edited",
            "assistant: Done."
        ]
    );
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn up_after_the_agent_accepted_the_draft_changes_nothing() {
    let home = Directory::new();
    let directory = Directory::new();
    let (release, gate) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![
        gated(gate),
        Response::Json(200, completion("Done.", vec![])),
    ]);
    let mut app = app(&home, &directory, &fixture);
    start(&mut app, "active");
    submit(&mut app, "joined");
    release.send(()).unwrap();
    wait_until(&mut app, |app| {
        rows(app).contains(&"user: joined".to_owned())
    });
    key(&mut app, 38, 0);
    assert!(app.state.editor.text.is_empty());
    assert!(app.state.queue.is_empty());
    wait_until(&mut app, |app| app.worker.is_none());
    assert_eq!(app.state.history.snapshot(), ["active", "joined"]);
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn recovery_keeps_queued_input_before_a_newer_edited_history_recall() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut current = app(&home, &directory, &fixture);
    current
        .agent
        .as_mut()
        .unwrap()
        .prepare_turn("first request")
        .unwrap();
    current.state.history.record("earlier prompt");
    current.state.editor.insert("unsent draft");
    current.state.editor.cursor = 3;
    current.state.queue.push("queued first").unwrap();
    key(&mut current, 80, 4);
    current.input(Decoded::Text(" revised".into())).unwrap();
    current.persist_input(true);
    let id = current.persistence.as_ref().unwrap().id();
    drop(current);

    let mut resumed = app(&home, &directory, &fixture);
    resumed.resume_session(&id);
    assert_eq!(resumed.state.editor.text, "unsent draft");
    assert_eq!(resumed.state.editor.cursor, 3);
    assert_eq!(
        resumed.state.queue.pending(),
        Some(("queued first\n\nearlier prompt revised".into(), true))
    );
    resumed.poll().unwrap();
    assert!(resumed.worker.is_none());
    assert!(fixture.finish().is_empty());
}

#[test]
fn stopping_with_an_empty_composer_returns_the_draft_to_it() {
    let home = Directory::new();
    let directory = Directory::new();
    let (release, gate) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![gated(gate)]);
    let mut app = app(&home, &directory, &fixture);
    start(&mut app, "active");
    submit(&mut app, "queued");
    key(&mut app, 27, 0);
    release.send(()).unwrap();
    wait_until(&mut app, |app| app.worker.is_none());
    assert_eq!(app.state.editor.text, "queued");
    assert!(app.state.queue.is_empty());
    assert!(
        !app.state
            .ending
            .as_ref()
            .is_some_and(|ending| ending.text.contains("draft paused"))
    );
    assert_eq!(app.state.history.snapshot(), ["active"]);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn stopping_with_text_in_the_composer_keeps_the_draft_paused() {
    let home = Directory::new();
    let directory = Directory::new();
    let (release, gate) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![gated(gate)]);
    let mut app = app(&home, &directory, &fixture);
    start(&mut app, "active");
    submit(&mut app, "queued");
    app.input(Decoded::Text("typing".into())).unwrap();
    key(&mut app, 27, 0);
    release.send(()).unwrap();
    wait_until(&mut app, |app| app.worker.is_none());
    assert_eq!(app.state.editor.text, "typing");
    assert_eq!(app.state.queue.pending(), Some(("queued".into(), true)));
    assert!(
        app.state
            .ending
            .as_ref()
            .is_some_and(|ending| ending.text.contains("draft paused"))
    );
    app.capture_input();
    let input = app.persistence.as_ref().unwrap().snapshot().input;
    assert_eq!(input.paused.unwrap().text, "queued");
    // A paused draft never sends itself; Up brings it back.
    app.state.editor.take();
    key(&mut app, 38, 0);
    assert_eq!(app.state.editor.text, "queued");
    assert!(app.state.queue.is_empty());
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn a_message_sent_after_the_turn_closed_starts_the_next_turn_exactly_once() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("First.", vec![])),
        (200, completion("Second.", vec![])),
    ]);
    let mut app = app(&home, &directory, &fixture);
    submit(&mut app, "active");
    worker_finished(&app);
    // The turn has closed, but the interface has not observed it yet.
    submit(&mut app, "after");
    assert_eq!(app.state.queue.messages(), ["after"]);
    wait_until(&mut app, |app| {
        app.worker.is_none() && app.state.queue.is_empty()
    });
    assert_eq!(
        rows(&app),
        [
            "user: active",
            "assistant: First.",
            "user: after",
            "assistant: Second.",
        ]
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(super::tests::task_content(&requests[1]), "after");
    assert!(super::tests::runtime_state(&requests[1]).contains("Permissions:"));
}

#[test]
fn a_save_failure_returns_the_message_to_the_composer() {
    let home = Directory::new();
    let directory = Directory::new();
    let (release, gate) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![gated(gate)]);
    let mut app = app(&home, &directory, &fixture);
    start(&mut app, "active");
    submit(&mut app, "queued");
    let handle = app.persistence.clone().unwrap();
    let bucket = std::fs::read_dir(home.path().join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let path = bucket.join(format!("{}.jsonl", handle.id()));
    let original = std::fs::metadata(&path).unwrap().permissions();
    let mut permissions = original.clone();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions).unwrap();
    release.send(()).unwrap();
    wait_until(&mut app, |app| app.worker.is_none());
    std::fs::set_permissions(&path, original).unwrap();
    // The composer was empty, so the message returns to it.
    assert_eq!(app.state.editor.text, "queued");
    assert!(app.state.queue.is_empty());
    assert_eq!(app.state.history.snapshot(), ["active"]);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn a_resumed_session_shows_a_joined_message_once_without_a_pending_copy() {
    let home = Directory::new();
    let directory = Directory::new();
    let (release, gate) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![
        gated(gate),
        Response::Json(200, completion("Done.", vec![])),
    ]);
    let mut first = app(&home, &directory, &fixture);
    start(&mut first, "active");
    submit(&mut first, "joined");
    release.send(()).unwrap();
    wait_until(&mut first, |app| app.worker.is_none());
    first.persist_input(true);
    let id = first.persistence.as_ref().unwrap().id();
    drop(first);
    let unused = HttpFixture::new(vec![]);
    let mut resumed = app(&home, &directory, &unused);
    resumed.resume_session(&id);
    assert_eq!(
        rows(&resumed),
        [
            "user: active",
            "assistant: Active.",
            "user: joined",
            "assistant: Done."
        ]
    );
    assert!(resumed.state.queue.is_empty());
    assert_eq!(fixture.finish().len(), 2);
    assert!(unused.finish().is_empty());
}
