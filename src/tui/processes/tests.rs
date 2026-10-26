use super::super::*;
use super::{Logs, Panel, RETAINED, Stream};
use crate::{
    backend::openrouter::OpenRouter,
    json::Value,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::{
        Tools,
        processes::{Page, Process},
    },
};
use std::{thread, time::Instant};

mod control_tests;
mod credential_tests;
mod view_tests;

const UP: u16 = 38;
const ESC: u16 = 27;

fn app(directory: &Directory, fixture: &HttpFixture) -> App {
    let mut app = App::new(
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        ),
        tests::config(directory),
        None,
    );
    app.state.width = 80;
    app.state.height = 30;
    app
}

fn key(app: &mut App, code: u16, modifiers: u8) {
    assert!(
        !app.input(Decoded::Key(terminal::Key {
            code,
            modifiers,
            character: 0
        }))
        .unwrap()
    );
}

fn submit(app: &mut App, value: &str) {
    app.input(Decoded::Text(value.into())).unwrap();
    key(app, 13, 0);
}

#[track_caller]
fn wait_until(app: &mut App, mut done: impl FnMut(&mut App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(app) {
        app.poll().unwrap();
        assert!(
            Instant::now() < deadline,
            "Timed out while {}",
            app.state.status
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn panel(app: &App) -> &Panel {
    app.state.processes.as_ref().unwrap()
}

fn logs(app: &App) -> &Logs {
    panel(app).logs.as_ref().unwrap()
}

fn stream(app: &App) -> &Stream {
    let logs = logs(app);
    &logs.views[logs.stream]
}

fn frame(app: &App) -> Vec<String> {
    view::composer_frame(&app.state, "fixture/model", "fixture directory")
        .live
        .iter()
        .map(|line| line.plain())
        .collect()
}

fn bash(id: &str, command: &str, persistent: bool) -> Value {
    let arguments = if persistent {
        Value::object([
            ("command", Value::string(command)),
            ("wait_ms", Value::number(0)),
        ])
    } else {
        Value::object([("command", Value::string(command))])
    };
    tool_call(id, "bash", arguments)
}

/// A turn that launches a long-output process, then holds the worker until
/// the `turn` file exists.
fn running_turn() -> HttpFixture {
    HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    bash(
                        "launch",
                        "seq 1 30000; printf 'err\\n' >&2; until [ -e more ]; do sleep 0.02; done; echo tail-line; until [ -e stop ]; do sleep 0.02; done",
                        true,
                    ),
                    bash("hold", "until [ -e turn ]; do sleep 0.02; done", false),
                ],
            ),
        ),
        (200, completion("Done.", vec![])),
    ])
}

fn create(directory: &Directory, name: &str) {
    std::fs::write(directory.path().join(name), "").unwrap();
}

#[test]
fn the_panel_opens_and_follows_output_while_the_worker_owns_the_agent() {
    let directory = Directory::new();
    let fixture = running_turn();
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Start the fixture.");
    let history = app.state.history.snapshot();
    let observer = app.process_observer.clone();
    wait_until(&mut app, |_| {
        observer.processes().is_some_and(|list| !list.is_empty())
    });
    let id = observer.processes().unwrap()[0].id.clone();
    wait_until(&mut app, |_| {
        observer
            .read(&id, 1, crate::tools::processes::Window::Tail)
            .is_ok_and(|page| page.text.ends_with("err\n"))
            && observer
                .read(&id, 0, crate::tools::processes::Window::Tail)
                .is_ok_and(|page| page.text.ends_with("29999\n30000\n"))
    });
    // Bash may add startup warnings; retain the entire stream for comparison.
    let errors = observer
        .read(&id, 1, crate::tools::processes::Window::Tail)
        .unwrap()
        .text;
    assert!(app.agent.is_none() && app.worker.is_some());
    let queued = app.state.queue.len();

    // Opening is local and immediate: nothing is queued for the model.
    submit(&mut app, "/terminal");
    assert_eq!(app.state.queue.len(), queued);
    assert!(app.state.editor.prompt().text.is_empty());
    assert_eq!(app.state.history.snapshot(), history);
    assert_eq!(panel(&app).processes.len(), 1);
    assert_eq!(panel(&app).processes[0].label(), "running");
    assert!(frame(&app).iter().any(|line| line.contains("seq 1 30000")));

    // Typing is ignored rather than reaching the hidden composer.
    app.input(Decoded::Text("ignored".into())).unwrap();
    app.input(Decoded::Paste("ignored".into())).unwrap();
    assert!(app.state.editor.prompt().text.is_empty());

    key(&mut app, 13, 0);
    assert_eq!(logs(&app).id, id);
    assert!(stream(&app).follow);
    assert!(stream(&app).text().ends_with("29999\n30000\n"));
    assert!(stream(&app).pages[0].start > 0);
    key(&mut app, 9, 0);
    assert_eq!(logs(&app).stream, 1);
    assert_eq!(stream(&app).text(), errors);
    assert!(frame(&app).iter().any(|line| line.contains("err")));
    key(&mut app, 9, 0);
    assert_eq!(logs(&app).stream, 0);

    // Scrolling back pauses following; new output does not move the view.
    key(&mut app, UP, 0);
    let paused = stream(&app).offset;
    assert!(!stream(&app).follow);
    create(&directory, "more");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !observer
        .read(&id, 0, crate::tools::processes::Window::Tail)
        .unwrap()
        .text
        .ends_with("tail-line\n")
    {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    app.refresh_processes(true);
    assert_eq!(stream(&app).offset, paused);
    assert!(!stream(&app).text().contains("tail-line"));
    key(&mut app, 35, 0);
    assert!(stream(&app).follow);
    assert!(stream(&app).text().ends_with("30000\ntail-line\n"));

    // Esc and Ctrl+C step back through the panel without touching the worker.
    key(&mut app, ESC, 0);
    assert!(panel(&app).logs.is_none());
    key(&mut app, 67, 4);
    assert!(app.state.processes.is_none());
    assert!(app.worker.is_some());
    assert!(observer.processes().unwrap()[0].active());

    create(&directory, "turn");
    wait_until(&mut app, |app| app.worker.is_none());
    assert!(
        app.state
            .rows()
            .iter()
            .flatten()
            .any(|row| row.plain() == "● Done.")
    );
    assert_eq!(fixture.finish().len(), 2);

    // The viewer read nothing on the model's behalf: its first poll starts at 0.
    let poll = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "poll",
                    "process",
                    Value::object([
                        ("action", Value::string("poll")),
                        ("process_id", Value::string(&id)),
                    ]),
                )],
            ),
        ),
        (200, completion("Polled.", vec![])),
    ]);
    app.agent
        .as_mut()
        .unwrap()
        .replace_client(OpenRouter::fixture(poll.endpoint.clone()));
    submit(&mut app, "Poll.");
    wait_until(&mut app, |app| app.worker.is_none());
    let result = app
        .archive
        .messages
        .lock()
        .unwrap()
        .iter()
        .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some("poll"))
        .and_then(|message| message.get("content").and_then(Value::as_str))
        .map(|content| crate::json::parse(content).unwrap())
        .unwrap();
    // The observer did not advance the model cursor. Its first bounded poll
    // starts at the beginning and leaves the rest available to later polls.
    let offset = result.get("stdout_offset").and_then(Value::as_usize);
    let skipped = result.get("stdout_skipped_bytes").and_then(Value::as_usize);
    assert_eq!(offset, Some(0));
    assert_eq!(skipped, Some(0));
    assert!(text_of(&result, "stdout").starts_with("1\n2\n"));
    assert_eq!(
        result.get("stdout_next_offset").and_then(Value::as_usize),
        Some(text_of(&result, "stdout").len())
    );
    assert_eq!(text_of(&result, "stderr"), errors);
    assert_eq!(poll.finish().len(), 2);
    create(&directory, "stop");
}

fn text_of(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap().to_string()
}

#[test]
fn scrolling_loads_bounded_older_and_newer_pages() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion("", vec![bash("launch", "seq 1 30000", true)]),
        ),
        (200, completion("Started.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Start.");
    wait_until(&mut app, |app| app.worker.is_none());
    wait_until(&mut app, |app| {
        app.process_observer.processes().unwrap()[0].label() == "exited 0"
    });
    let id = app.process_observer.processes().unwrap()[0].id.clone();
    submit(&mut app, &format!("/terminal {id}"));
    assert_eq!(logs(&app).id, id);
    let length = stream(&app).length();
    assert!(length > RETAINED);

    // Older pages load at the top edge and keep the window bounded.
    for _ in 0..4 {
        app.scroll_logs(-100_000);
    }
    let top = stream(&app);
    assert_eq!((top.start(), top.offset), (0, 0));
    assert!(top.end() - top.start() <= RETAINED);
    assert!(top.text().starts_with("1\n2\n3\n"));
    assert!(!top.follow);

    // Newer pages load at the bottom edge; reaching the end resumes following.
    for _ in 0..8 {
        app.scroll_logs(100_000);
    }
    let bottom = stream(&app);
    assert_eq!(bottom.end(), length);
    assert!(bottom.end() - bottom.start() <= RETAINED);
    assert!(bottom.follow);

    key(&mut app, 36, 0);
    assert_eq!((stream(&app).start(), stream(&app).offset), (0, 0));
    assert!(
        frame(&app)
            .iter()
            .any(|line| line.split_whitespace().next() == Some("1"))
    );
    key(&mut app, 35, 0);
    assert!(stream(&app).follow);
    assert!(stream(&app).text().ends_with("30000\n"));
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn a_new_conversation_closes_the_viewer_and_stops_its_processes() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![bash(
                    "launch",
                    "until [ -e never ]; do sleep 0.02; done",
                    true,
                )],
            ),
        ),
        (200, completion("Started.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Start.");
    wait_until(&mut app, |app| app.worker.is_none());
    let old = app.process_observer.clone();
    let id = old.processes().unwrap()[0].id.clone();
    submit(&mut app, "/terminal");
    key(&mut app, 13, 0);
    assert_eq!(logs(&app).id, id);

    app.dispatch("/new".into()).unwrap();
    wait_until(&mut app, |app| app.state.processes.is_none());
    // The viewer kept neither the old conversation nor its process alive.
    assert!(old.processes().is_none());
    submit(&mut app, &format!("/terminal {id}"));
    assert!(panel(&app).processes.is_empty());
    assert!(panel(&app).logs.is_none());
    assert!(!app.process_observer.same(&old));
    assert_eq!(fixture.finish().len(), 2);
}
