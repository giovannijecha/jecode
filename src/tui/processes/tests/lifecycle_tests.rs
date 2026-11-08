//! Completions in the conversation, the footer, idle continuation and moving
//! a waiting command into the background.

use super::*;
use crate::tui::state::Item;

const GATED: &str = "printf ready; until [ -e go ]; do sleep 0.02; done; printf done";

fn rows(app: &App) -> String {
    app.state
        .rows()
        .iter()
        .flatten()
        .map(|row| row.plain())
        .collect::<Vec<_>>()
        .join("\n")
}

fn footer(app: &App) -> String {
    frame(app).last().cloned().unwrap_or_default()
}

/// Completion rows in the conversation.
fn completion_rows(app: &App) -> usize {
    app.state
        .items
        .iter()
        .filter(|item| matches!(item, Item::Tool { id, .. } if id.starts_with("process:")))
        .count()
}

fn poll_a_while(app: &mut App) {
    for _ in 0..10 {
        app.poll().unwrap();
        thread::sleep(Duration::from_millis(10));
    }
}

fn delivered(body: &Value) -> bool {
    body.get("messages")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .any(|message| {
            message
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains("Processes that finished"))
        })
}

fn session_app(home: &Directory, directory: &Directory, fixture: &HttpFixture) -> App {
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let mut app = App::new(agent, tests::config(directory), None);
    app.state.width = 80;
    app.state.height = 30;
    tests::full_app(app)
}

fn saved_completions(app: &App) -> usize {
    app.persistence
        .as_ref()
        .unwrap()
        .snapshot()
        .records()
        .into_iter()
        .filter(|record| matches!(record, crate::sessions::Record::ProcessFinished { .. }))
        .count()
}

fn item_index(app: &App, id: &str) -> usize {
    app.state
        .items
        .iter()
        .position(|item| matches!(item, Item::Tool { id: current, .. } if current == id))
        .unwrap()
}

#[test]
fn a_completion_during_another_call_restores_after_that_call() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    bash("start", GATED, true),
                    bash("hold", "until [ -e release ]; do sleep 0.02; done", false),
                ],
            ),
        ),
        (200, completion("Done.", vec![])),
    ]);
    let mut app = session_app(&home, &directory, &fixture);
    submit(&mut app, "Build.");
    wait_until(&mut app, |app| {
        app.state
            .items
            .iter()
            .any(|item| matches!(item, Item::Tool { id, result: None, .. } if id == "hold"))
    });
    create(&directory, "go");
    wait_until(&mut app, |app| completion_rows(app) == 1);
    assert!(app.state.notice.is_none());
    let status = frame(&app).join("\n");
    assert!(status.contains("Esc interrupt"), "{status}");
    let process_row = app
        .state
        .items
        .iter()
        .find_map(|item| match item {
            Item::Tool { id, .. } if id.starts_with("process:") => Some(id.clone()),
            _ => None,
        })
        .unwrap();
    assert!(item_index(&app, "hold") < item_index(&app, &process_row));
    create(&directory, "release");
    wait_until(&mut app, |app| app.worker.is_none());
    let id = app.persistence.as_ref().unwrap().id();
    drop(app);

    let mut resumed = session_app(&home, &directory, &fixture);
    resumed.resume_session(&id);
    assert!(item_index(&resumed, "hold") < item_index(&resumed, &process_row));
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn a_completion_shown_behind_a_menu_survives_resume_without_replay() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![bash("start", GATED, true)])),
        (200, completion("Started.", vec![])),
    ]);
    let mut app = session_app(&home, &directory, &fixture);
    submit(&mut app, "Build.");
    wait_until(&mut app, |app| app.worker.is_none());
    submit(&mut app, "/settings");
    assert!(app.state.selector.is_some());
    create(&directory, "go");
    wait_until(&mut app, |app| completion_rows(app) == 1);
    assert_eq!(saved_completions(&app), 1);
    assert!(
        app.persistence
            .as_ref()
            .unwrap()
            .snapshot()
            .events
            .iter()
            .any(
                |event| event.get("type").and_then(Value::as_str) == Some("process_finished")
                    && event.get("in_turn") == Some(&Value::Bool(false))
            )
    );
    assert!(app.agent.as_ref().unwrap().can_continue_processes());
    let id = app.persistence.as_ref().unwrap().id();
    drop(app);

    let mut resumed = session_app(&home, &directory, &fixture);
    resumed.resume_session(&id);
    assert_eq!(completion_rows(&resumed), 1);
    assert!(rows(&resumed).contains(GATED));
    assert_eq!(saved_completions(&resumed), 1);
    assert!(!resumed.agent.as_ref().unwrap().can_continue_processes());
    assert!(resumed.worker.is_none());
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn a_rejected_provider_retry_keeps_one_saved_completion() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![bash("start", GATED, true)])),
        (200, completion("Started.", vec![])),
        (
            503,
            Value::object([("error", Value::string("temporarily unavailable"))]),
        ),
        (200, completion("It passed.", vec![])),
    ]);
    let mut app = session_app(&home, &directory, &fixture);
    submit(&mut app, "Build.");
    wait_until(&mut app, |app| app.worker.is_none());
    create(&directory, "go");
    wait_until(&mut app, |app| {
        app.worker.is_none() && rows(app).contains("It passed.")
    });
    assert_eq!(completion_rows(&app), 1);
    assert_eq!(saved_completions(&app), 1);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 4);
    assert!(delivered(&requests[2].body));
    assert!(delivered(&requests[3].body));
}

#[test]
fn an_idle_conversation_shows_the_outcome_once_and_continues() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![bash("start", GATED, true)])),
        (200, completion("Started.", vec![])),
        (200, completion("It passed.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Build.");
    wait_until(&mut app, |app| app.worker.is_none());
    app.poll().unwrap();
    let running = footer(&app);
    assert!(running.contains("PROCESS  1 running"), "{running}");
    assert!(running.contains("/process"), "{running}");
    assert_eq!(completion_rows(&app), 0);

    // A draft waits untouched while the conversation continues.
    app.state.editor.insert("half a thought");
    create(&directory, "go");
    wait_until(&mut app, |app| {
        app.worker.is_none() && rows(app).contains("It passed.")
    });
    assert_eq!(app.state.editor.prompt().text, "half a thought");
    assert_eq!(completion_rows(&app), 1);
    let transcript = rows(&app);
    assert!(transcript.contains("PROCESS"), "{transcript}");
    assert!(transcript.contains(GATED), "{transcript}");
    assert!(!footer(&app).contains("PROCESS"));

    poll_a_while(&mut app);
    assert_eq!(completion_rows(&app), 1);
    // The turn it started keeps the row in Ctrl+O's range.
    assert_eq!(
        app.state.toggle_tools().map(|(expanded, _)| expanded),
        Some(true)
    );
    let row = app.state.items.iter().find_map(|item| match item {
        Item::Tool {
            id, presentation, ..
        } if id.starts_with("process:") => Some(presentation),
        _ => None,
    });
    assert!(row.unwrap().expanded);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert!(delivered(&requests[2].body));
}

#[test]
fn an_explicit_stop_is_not_undone_by_a_process_that_finishes_later() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    bash("start", GATED, true),
                    bash("hold", "until [ -e never ]; do sleep 0.02; done", false),
                ],
            ),
        ),
        (200, completion("Heard you, and it finished.", vec![])),
    ]);
    let mut app = session_app(&home, &directory, &fixture);
    submit(&mut app, "Build.");
    let observer = app.process_observer.clone();
    wait_until(&mut app, |_| {
        observer.processes().is_some_and(|list| list.len() == 2)
    });
    // Esc stops the turn and the foreground call, and leaves the background one.
    key(&mut app, ESC, 0);
    wait_until(&mut app, |app| app.worker.is_none());
    assert!(observer.processes().unwrap().iter().any(|p| p.active()));
    assert!(!app.process_auto_resume);

    create(&directory, "go");
    let deadline = Instant::now() + Duration::from_secs(10);
    while observer.completions_since(0).1.is_empty() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    app.agent.as_ref().unwrap().save_session().unwrap();
    let saved = app.persistence.as_ref().unwrap().snapshot();
    let event = saved
        .events
        .iter()
        .find(|event| event.get("type").and_then(Value::as_str) == Some("process_finished"));
    assert_eq!(
        event.and_then(|event| event.get("in_turn")),
        Some(&Value::Bool(false))
    );
    wait_until(&mut app, |app| completion_rows(app) == 1);
    assert_eq!(saved_completions(&app), 1);
    let process_row = app
        .state
        .items
        .iter()
        .position(|item| matches!(item, Item::Tool { id, .. } if id.starts_with("process:")))
        .unwrap();
    assert!(!crate::tui::state::rows::in_chapter(
        &app.state.items,
        process_row
    ));
    poll_a_while(&mut app);
    assert!(
        app.worker.is_none(),
        "the completion must not restart the turn"
    );

    // The next message carries it.
    submit(&mut app, "What happened?");
    assert!(app.process_auto_resume);
    wait_until(&mut app, |app| {
        app.worker.is_none() && rows(app).contains("Heard you")
    });
    assert_eq!(saved_completions(&app), 1);
    poll_a_while(&mut app);
    let id = app.persistence.as_ref().unwrap().id();
    drop(app);
    let mut resumed = session_app(&home, &directory, &fixture);
    resumed.resume_session(&id);
    let process_row = resumed
        .state
        .items
        .iter()
        .position(|item| matches!(item, Item::Tool { id, .. } if id.starts_with("process:")))
        .unwrap();
    assert_eq!(completion_rows(&resumed), 1);
    assert!(
        !crate::tui::state::rows::in_chapter(&resumed.state.items, process_row),
        "a completion outside a turn must stay a lone row after resume: {}",
        rows(&resumed)
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(!delivered(&requests[0].body));
    assert!(delivered(&requests[1].body));
}

#[test]
fn a_pending_menu_holds_the_continuation_until_it_closes() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![bash("start", GATED, true)])),
        (200, completion("Started.", vec![])),
        (200, completion("It passed.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Build.");
    wait_until(&mut app, |app| app.worker.is_none());
    submit(&mut app, "/settings");
    assert!(app.state.selector.is_some());
    create(&directory, "go");
    wait_until(&mut app, |app| completion_rows(app) == 1);
    poll_a_while(&mut app);
    assert!(app.state.selector.is_some());
    assert!(app.worker.is_none(), "a menu is waiting for the user");

    key(&mut app, ESC, 0);
    assert!(app.state.selector.is_none());
    wait_until(&mut app, |app| {
        app.worker.is_none() && rows(app).contains("It passed.")
    });
    assert_eq!(fixture.finish().len(), 3);
}

#[test]
fn a_waiting_command_moves_to_the_background_without_a_second_launch() {
    let directory = Directory::new();
    let command = "printf 'run\\n' >> runs; until [ -e go ]; do sleep 0.02; done";
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![bash("wait", command, false)])),
        (200, completion("It keeps running.", vec![])),
        (200, completion("It finished.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    app.agent
        .as_ref()
        .unwrap()
        .set_process_wait(Duration::from_secs(60));
    submit(&mut app, "Run it.");
    app.state.editor.insert("my draft");
    let observer = app.process_observer.clone();
    wait_until(&mut app, |_| {
        observer
            .processes()
            .is_some_and(|list| list.iter().any(|process| process.awaited))
    });
    wait_until(&mut app, |_| directory.path().join("runs").exists());
    let id = observer.processes().unwrap()[0].id.clone();
    assert!(
        footer(&app).contains("fixture"),
        "not yet a background process"
    );
    assert!(!footer(&app).contains("PROCESS"));

    app.open_processes("");
    let list = frame(&app).join("\n");
    assert!(list.contains("b background"), "{list}");
    assert_eq!(panel(&app).selected_process().unwrap().id, id);
    // A paste is text, not the key.
    app.input(Decoded::Paste("b".into())).unwrap();
    assert!(observer.processes().unwrap()[0].awaited);
    app.input(Decoded::Text("b".into())).unwrap();
    wait_until(&mut app, |app| {
        app.worker.is_none() && rows(app).contains("It keeps running.")
    });
    assert_eq!(app.state.editor.prompt().text, "my draft");
    assert!(observer.processes().unwrap()[0].active());
    app.refresh_processes(true);
    let list = frame(&app).join("\n");
    assert!(!list.contains("b background"), "{list}");
    assert_eq!(
        std::fs::read_to_string(directory.path().join("runs")).unwrap(),
        "run\n"
    );
    // Pressing it again with nothing waiting changes nothing.
    app.input(Decoded::Text("b".into())).unwrap();
    assert!(app.state.processes.is_some());

    create(&directory, "go");
    wait_until(&mut app, |app| {
        app.worker.is_none() && rows(app).contains("It finished.")
    });
    assert_eq!(
        std::fs::read_to_string(directory.path().join("runs")).unwrap(),
        "run\n"
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert!(delivered(&requests[2].body));
}

#[test]
fn viewing_a_process_never_touches_the_conversation() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion("", vec![bash("short", "printf local", false)]),
        ),
        (200, completion("Done.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Run it.");
    wait_until(&mut app, |app| app.worker.is_none());
    let messages = app.archive.messages.lock().unwrap().len();
    let history = app.state.history.snapshot();
    app.state.editor.insert("kept");
    app.open_processes("");
    assert_eq!(panel(&app).processes.len(), 1);
    key(&mut app, 13, 0);
    assert_eq!(stream(&app).text(), "local");
    key(&mut app, ESC, 0);
    key(&mut app, ESC, 0);
    assert!(app.state.processes.is_none());
    assert_eq!(app.state.editor.prompt().text, "kept");
    assert_eq!(app.state.history.snapshot(), history);
    assert_eq!(app.archive.messages.lock().unwrap().len(), messages);
    assert_eq!(completion_rows(&app), 0, "the receipt already showed it");
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn the_previous_command_name_is_gone() {
    use crate::session::commands;
    assert!(
        commands::COMMANDS
            .iter()
            .any(|command| command.name == "/process")
    );
    assert!(
        !commands::COMMANDS
            .iter()
            .any(|command| command.name == "/terminal")
    );
    assert!(commands::help(true).contains("/process [ID]"));
    assert!(commands::help(false).contains("/process write ID TEXT"));
    assert!(!commands::help(true).contains("/terminal"));
    assert!(!commands::help(false).contains("/terminal"));
}

mod outcome_tests;
