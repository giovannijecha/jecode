use super::*;
use crate::backend::openrouter::OpenRouter;
use crate::config::{Settings, Store};
use crate::json::{self, Value};
use crate::test_support::{Directory, HttpFixture, completion, tool_call};
use crate::tools::Tools;
use std::fs;
use std::thread;
use std::time::Instant;

mod arrival_tests;
mod flow_tests;
mod native_smoke;
mod project_instruction_tests;
mod quit_tests;
mod work_tests;
mod work_todo_tests;

pub(super) fn config(directory: &Directory) -> SessionConfig {
    SessionConfig {
        store: Store::new(directory.path().join(".jecode")),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
    }
}
// These process fixtures exercise unrestricted Bash; permissions have dedicated tests.
pub(super) fn full_app(mut app: App) -> App {
    app.config.settings.default_permissions = crate::permissions::Level::Full;
    app.permission_observer
        .set_default(crate::permissions::Level::Full);
    app.permission_observer
        .set_level(crate::permissions::Level::Full)
        .unwrap();
    app.refresh_permission();
    app
}
fn app(directory: &Directory, fixture: &HttpFixture) -> App {
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    full_app(App::new(agent, config(directory), None))
}
fn submit(app: &mut App, text: &str) {
    app.state.editor.insert(text);
    app.edited();
    assert!(!app.submit().unwrap());
}
pub(super) fn finish(app: &mut App) {
    let started = Instant::now();
    while app.worker.is_some() {
        app.poll().unwrap();
        assert!(started.elapsed() < Duration::from_secs(6));
        thread::sleep(Duration::from_millis(10));
    }
}
/// Polls until `ready` holds; the worker thread advances independently.
pub(super) fn wait_until(app: &mut App, ready: impl Fn(&App) -> bool) {
    let started = Instant::now();
    while !ready(app) {
        app.poll().unwrap();
        assert!(started.elapsed() < Duration::from_secs(6));
        thread::sleep(Duration::from_millis(5));
    }
}
/// Conversation messages in transcript order, as `role: text`.
pub(super) fn rows(app: &App) -> Vec<String> {
    app.state
        .items
        .iter()
        .filter_map(|item| match item {
            state::Item::Text {
                kind: Kind::User,
                text,
            } => Some(format!("user: {text}")),
            state::Item::Text {
                kind: Kind::Assistant,
                text,
            } => Some(format!("assistant: {text}")),
            _ => None,
        })
        .collect()
}
/// The last durable user task, followed by separate request-local runtime state.
pub(super) fn task_content(request: &crate::test_support::Request) -> String {
    let messages = request.body.get("messages").unwrap().as_array().unwrap();
    messages
        .iter()
        .rev()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .find(|message| {
            !message
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|content| {
                    content.starts_with("Jecode session state for the next request.")
                })
        })
        .unwrap()
        .get("content")
        .and_then(Value::as_str)
        .unwrap()
        .into()
}

pub(super) fn runtime_state(request: &crate::test_support::Request) -> String {
    let messages = request.body.get("messages").unwrap().as_array().unwrap();
    messages
        .last()
        .unwrap()
        .get("content")
        .and_then(Value::as_str)
        .filter(|content| content.starts_with("Jecode session state for the next request."))
        .unwrap()
        .into()
}

#[test]
fn worker_results_previews_export_and_local_commands_use_the_same_conversation() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "Inspecting.",
                vec![
                    tool_call(
                        "write-1",
                        "write",
                        Value::object([
                            ("path", Value::string("fixture.txt")),
                            ("content", Value::string("before\nisolated-fixture-key")),
                        ]),
                    ),
                    tool_call(
                        "read-1",
                        "read",
                        Value::object([("path", Value::string("fixture.txt"))]),
                    ),
                    tool_call(
                        "edit-1",
                        "edit",
                        Value::object([
                            ("path", Value::string("fixture.txt")),
                            ("old_text", Value::string("before")),
                            ("new_text", Value::string("after")),
                        ]),
                    ),
                    tool_call(
                        "bash-1",
                        "bash",
                        Value::object([(
                            "command",
                            Value::string("cat fixture.txt; printf 'problem' >&2; exit 7"),
                        )]),
                    ),
                ],
            ),
        ),
        (200, completion("Finished, with exit 7.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Inspect the fixture.");
    app.input(Decoded::Key(terminal::Key {
        code: 79,
        modifiers: 4,
        character: 15,
    }))
    .unwrap();
    submit(&mut app, "/export"); // A snapshot can also be exported while a turn is running.
    finish(&mut app);
    assert_eq!(app.state.status, "Ready");
    assert!(
        app.state
            .rows()
            .iter()
            .flatten()
            .any(|row| row.plain() == "● Finished, with exit 7.")
    );
    let rows = app.state.rows();
    assert!(
        rows.iter()
            .flatten()
            .any(|row| row.plain().contains("problem"))
    );
    assert!(
        rows.iter()
            .flatten()
            .any(|row| row.plain().contains("[redacted]"))
    );
    assert!(
        !rows
            .iter()
            .flatten()
            .any(|row| row.plain().contains("isolated-fixture-key"))
    );
    let path = app.archive.save().unwrap();
    let text = fs::read_to_string(path).unwrap();
    let document = json::parse(&text).unwrap();
    let messages = document.get("messages").unwrap().as_array().unwrap();
    assert_eq!(messages.len(), 8);
    assert!(text.contains("fixture-reasoning"));
    assert!(!text.contains("isolated-fixture-key"));
    assert_eq!(fixture.finish().len(), 2);
    submit(&mut app, "/clear");
    assert_eq!(
        app.archive
            .document()
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn one_tool_per_model_request_keeps_each_read_and_the_next_turn_preserves_it() {
    let directory = Directory::new();
    fs::write(
        directory.path().join("fixture.txt"),
        "first\nsecond\nthird\n",
    )
    .unwrap();
    let mut responses = (1..=3)
        .map(|offset| {
            (
                200,
                completion(
                    "",
                    vec![tool_call(
                        &format!("read-{offset}"),
                        "read",
                        Value::object([
                            ("path", Value::string("fixture.txt")),
                            ("offset", Value::number(offset)),
                            ("limit", Value::number(1)),
                        ]),
                    )],
                ),
            )
        })
        .collect::<Vec<_>>();
    responses.push((200, completion("Finished.", vec![])));
    responses.push((200, completion("Next request completed.", vec![])));
    let fixture = HttpFixture::new(responses);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Inspect three file lines.");
    finish(&mut app);
    assert_eq!(app.state.status, "Ready");
    let settled = app.state.rows();
    let headers = settled
        .iter()
        .flatten()
        .map(line::Line::plain)
        .filter(|row| row.starts_with(" WORK  Activity  "))
        .collect::<Vec<_>>();
    // Pages of one file share a chapter, drawn at the first read.
    assert_eq!(headers, [" WORK  Activity  3 calls"]);
    assert!(
        settled[1]
            .iter()
            .any(|row| row.plain().contains("│ 1 file read"))
    );
    assert!(
        !settled
            .iter()
            .flatten()
            .any(|row| row.plain().contains("More lines available"))
    );
    assert!(
        settled[3]
            .iter()
            .any(|row| row.plain().contains("READ fixture.txt")),
        "{:?}",
        settled
            .iter()
            .map(|rows| rows.iter().map(line::Line::plain).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    );
    app.input(Decoded::Key(terminal::Key {
        code: 79,
        modifiers: 4,
        character: 15,
    }))
    .unwrap();
    let expanded = app.state.rows();
    assert!(
        expanded[2]
            .iter()
            .any(|row| row.plain().contains("1 line · from line 2 · more from 3"))
    );
    app.input(Decoded::Key(terminal::Key {
        code: 79,
        modifiers: 4,
        character: 15,
    }))
    .unwrap();
    assert_eq!(app.state.rows(), settled);
    for (index, word) in ["first", "second", "third"].into_iter().enumerate() {
        let file_line = format!("{}: {word}", index + 1);
        assert!(
            !settled
                .iter()
                .flatten()
                .any(|row| row.plain().contains(&file_line))
        );
    }
    assert_eq!(
        view::frame(&app.state, "fixture", "folder").history.len(),
        5
    );
    submit(&mut app, "Another request.");
    finish(&mut app);
    assert_eq!(app.state.rows()[..5], settled);
    assert_eq!(fixture.finish().len(), 5);
}

#[test]
fn cancellation_keeps_matching_results_stops_pending_writes_and_allows_another_turn() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    tool_call(
                        "slow",
                        "bash",
                        Value::object([(
                            "command",
                            Value::string("printf 'started'; printf 'ready' > marker; sleep 20"),
                        )]),
                    ),
                    tool_call(
                        "pending",
                        "write",
                        Value::object([
                            ("path", Value::string("must-not-exist")),
                            ("content", Value::string("pending")),
                        ]),
                    ),
                ],
            ),
        ),
        (200, completion("Recovered.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Run the fixture.");
    let started = Instant::now();
    while !directory.path().join("marker").exists() {
        app.poll().unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(10));
    }
    submit(&mut app, "/export");
    app.worker.as_ref().unwrap().cancel();
    finish(&mut app);
    assert!(!directory.path().join("must-not-exist").exists());
    let document = app.archive.document();
    let messages = document.get("messages").unwrap().as_array().unwrap();
    assert_eq!(
        messages[3].get("tool_call_id").and_then(Value::as_str),
        Some("slow")
    );
    assert!(
        messages[3]
            .get("content")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("started")
    );
    assert_eq!(
        messages[4].get("tool_call_id").and_then(Value::as_str),
        Some("pending")
    );
    // The stopped turn returns the waiting command to the empty composer.
    assert_eq!(app.state.editor.text, "/export");
    assert!(app.state.queue.is_empty());
    app.state.editor.take();
    submit(&mut app, "Continue.");
    finish(&mut app);
    assert!(
        app.state
            .rows()
            .iter()
            .flatten()
            .any(|row| row.plain() == "● Recovered.")
    );
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn unused_expansion_and_native_scroll_keys_preserve_the_draft_and_conversation() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.editor.insert("first\nsecond");
    let cursor = app.state.editor.cursor;
    app.state
        .message(Kind::Assistant, &"previous response\n".repeat(40));
    let before = app.state.rows();
    for (code, modifiers, character) in [
        (79, 4, 15),
        (33, 0, 0),
        (34, 0, 0),
        (33, 6, 0),
        (34, 6, 0),
        (38, 6, 0),
        (40, 6, 0),
        (36, 6, 0),
        (35, 6, 0),
        (9, 0, 0),
        (113, 0, 0),
    ] {
        app.input(Decoded::Key(terminal::Key {
            code,
            modifiers,
            character,
        }))
        .unwrap();
        assert_eq!(app.state.editor.text, "first\nsecond");
        assert_eq!(app.state.editor.cursor, cursor);
        assert_eq!(app.state.rows(), before);
    }
    app.input(Decoded::Key(terminal::Key {
        code: 38,
        modifiers: 0,
        character: 0,
    }))
    .unwrap();
    assert_eq!(app.state.editor.cursor, "first".len());
    assert_eq!(
        app.archive
            .document()
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(fixture.finish().is_empty());
}
