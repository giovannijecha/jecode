use super::*;
use crate::config::{Settings, Store};
use crate::json::{self, Value};
use crate::openrouter::OpenRouter;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};
use crate::tools::Tools;
use std::fs;
use std::thread;
use std::time::Instant;

mod flow_tests;
mod project_instruction_tests;

pub(super) fn config(directory: &Directory) -> SessionConfig {
    SessionConfig {
        store: Store::new(directory.path().join(".jecode")),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    }
}
fn app(directory: &Directory, fixture: &HttpFixture) -> App {
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    App::new(agent, config(directory), None)
}
fn submit(app: &mut App, text: &str) {
    app.state.editor.insert(text);
    app.edited();
    assert!(!app.submit().unwrap());
}
fn finish(app: &mut App) {
    let started = Instant::now();
    while app.worker.is_some() {
        app.poll().unwrap();
        assert!(started.elapsed() < Duration::from_secs(6));
        thread::sleep(Duration::from_millis(10));
    }
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
            .any(|row| row.plain() == "Finished, with exit 7.")
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
fn one_tool_per_model_request_keeps_one_tree_and_the_next_turn_preserves_it() {
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
        .filter(|row| row.starts_with("├─ read") || row.starts_with("└─ read"))
        .collect::<Vec<_>>();
    assert_eq!(headers.len(), 3);
    for (index, header) in headers.iter().enumerate() {
        assert!(header.starts_with(if index == 2 {
            "└─ read  fixture.txt"
        } else {
            "├─ read  fixture.txt"
        }));
        assert!(header.contains(&format!("✓ lines {}–{}", index + 1, index + 1)));
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
    assert!(app.state.editor.text.is_empty());
    assert_eq!(app.state.queue.paused[0].text, "/export");
    app.state.editor.take();
    submit(&mut app, "Continue.");
    finish(&mut app);
    assert!(
        app.state
            .rows()
            .iter()
            .flatten()
            .any(|row| row.plain() == "Recovered.")
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

#[test]
#[ignore = "requires a real interactive Windows console and manual keyboard input"]
fn interactive_windows_smoke() {
    let directory = Directory::new();
    fs::write(
        directory.path().join("fixture.txt"),
        (1..=80)
            .map(|n| format!("fixture line {n}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let fixture = HttpFixture::with_wait(
        vec![
            (
                200,
                completion(
                    "I will inspect the fixture.",
                    vec![
                        tool_call(
                            "read-1",
                            "read",
                            Value::object([("path", Value::string("fixture.txt"))]),
                        ),
                    ],
                ),
            ),
            (
                200,
                completion(
                    "\n  \n",
                    vec![
                        tool_call(
                            "write-1",
                            "write",
                            Value::object([
                                ("path", Value::string("generated.rs")),
                                ("content", Value::string("fn fixture() {\n    println!(\"before\");\n}\n")),
                            ]),
                        ),
                    ],
                ),
            ),
            (
                200,
                completion(
                    "",
                    vec![
                        tool_call(
                            "edit-1",
                            "edit",
                            Value::object([
                                ("path", Value::string("generated.rs")),
                                ("old_text", Value::string("before")),
                                ("new_text", Value::string("after")),
                            ]),
                        ),
                    ],
                ),
            ),
            (
                200,
                completion(
                    "",
                    vec![
                        tool_call(
                            "bash-1",
                            "bash",
                            Value::object([(
                                "command",
                                Value::string(
                                    "printf 'stdout fixture\\n'; printf 'stderr fixture\\n' >&2; sleep 2",
                                ),
                            )]),
                        ),
                    ],
                ),
            ),
            (
                200,
                completion(
                    "",
                    vec![
                        tool_call("error-1", "bash", Value::object([
                            ("command", Value::string("printf 'fixture command failed\\n' >&2; exit 7")),
                        ])),
                    ],
                ),
            ),
            (200, completion(
                "# Tool fixture completed\nThe tool tree stays compact.\n\n```rust\nlet value = 1;\n\nprintln!(\"verified\");\n```\n\nSend a second task to inspect conversation scrolling.",
                vec![])),
            (200, completion(
                &format!("# Fixture verified\n\nAll four tools returned captured results.\nOne fixture command intentionally exited with **code 7**.\n\n- Inspect the compact tool previews above.\n- Scroll the conversation with the wheel or Page Up/Down.\n- Browse prompt history with Up/Down.\n\n```rust\nfn fixture() {{\n    let max_requests = 40;\n    println!(\"verified\"); // display fixture\n}}\n```\n\n{}",
                    "This later paragraph keeps the original tool cards in the conversation history.\n\n".repeat(30)),
                vec![])),
        ],
        Duration::from_secs(180),
    );
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    run(agent, config(&directory), Default::default()).unwrap();
    assert_eq!(fixture.finish().len(), 7);
    let exports: Vec<_> = fs::read_dir(directory.path())
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("JECODE-SESSION-")
        })
        .collect();
    assert!(!exports.is_empty(), "use /export during the smoke test");
    let text = fs::read_to_string(exports.last().unwrap().path()).unwrap();
    assert!(!text.contains("isolated-fixture-key"));
    assert!(json::parse(&text).is_ok());
    assert!(
        text.contains("fixture line 80"),
        "exports keep captured output beyond the preview"
    );
}
