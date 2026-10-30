//! Inside a collaborator: its edits with their hunks, live and after
//! resume, and rows drawn once until what they show changes.

use super::*;
use crate::test_support::tool_call;

fn app(home: &Directory, project: &Directory, endpoint: String) -> App {
    let mut agent = Agent::new(
        OpenRouter::fixture(endpoint),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let mut app = App::new(
        agent,
        SessionConfig {
            store: Store::new(home.path().join("config")),
            settings,
        },
        None,
    );
    app.state.width = 80;
    app.state.height = 60;
    app
}

/// Calls and events in order, by kind, chapter label and whether they
/// stand alone.
fn chapters(items: &[Item]) -> Vec<(String, String, bool)> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Tool {
                name,
                arguments,
                result,
                ..
            } => Some((
                if name == "collaboration" {
                    result
                        .as_ref()
                        .map_or("", |event| value(event, "kind"))
                        .into()
                } else {
                    format!("{name} {}", value(arguments, "action"))
                },
                value(arguments, "work").to_owned(),
                arguments.get("lone") == Some(&Value::Bool(true)),
            )),
            _ => None,
        })
        .collect()
}

/// Opens the first collaborator in `/agents` and returns its conversation.
fn inside(app: &mut App) -> String {
    app.open_agents();
    app.input(Decoded::Key(Key {
        code: 13,
        modifiers: 0,
        character: 0,
    }))
    .unwrap();
    text(&view::frame(&app.state, "fixture/model", "fixture").live)
}

#[test]
fn inside_edit_cards_show_the_child_hunks_live_and_after_resume() {
    let home = Directory::new();
    let project = Directory::new();
    let notes = project.path().join("notes.txt");
    std::fs::write(&notes, "first\nsecond\nthird\n").unwrap();
    let work = ("work", Value::string("Delegate the notes"));
    let create = Value::object([
        ("action", Value::string("create")),
        (
            "tasks",
            Value::Array(vec![Value::object([
                ("title", Value::string("Fix the notes")),
                ("instructions", Value::string("Replace the second line")),
                ("workspace", Value::string("shared")),
            ])]),
        ),
        work.clone(),
    ]);
    let wait = Value::object([
        ("action", Value::string("wait")),
        ("id", Value::string("a1")),
        work,
    ]);
    let edit = Value::object([
        ("path", Value::string("notes.txt")),
        ("old_text", Value::string("second")),
        ("new_text", Value::string("changed")),
    ]);
    // The main agent waits in the same batch, so the child's requests come
    // strictly between its own.
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    tool_call("create-1", "agent", create),
                    tool_call("wait-1", "agent", wait),
                ],
            ),
        ),
        (200, completion("", vec![tool_call("edit-1", "edit", edit)])),
        (200, completion("Replaced the second line", vec![])),
        (200, completion("The notes are fixed.", vec![])),
    ]);
    let mut live = app(&home, &project, fixture.endpoint.clone());
    live.collaboration_auto_resume = false;
    live.state.editor.insert("Fix the notes");
    live.submit().unwrap();
    let started = Instant::now();
    while live.worker.is_some() {
        live.poll().unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(5));
    }
    live.poll().unwrap();
    assert_eq!(
        std::fs::read_to_string(&notes).unwrap(),
        "first\nchanged\nthird\n"
    );
    let placed = chapters(&live.state.items);
    assert!(
        placed.contains(&("returned".into(), "Delegate the notes".into(), false)),
        "{placed:?}"
    );
    let shown = inside(&mut live);
    let changes = &live.state.agents_panel.as_ref().unwrap().changes;
    assert!(
        changes
            .calls
            .get("edit-1")
            .is_some_and(|change| !change.hunks.is_empty()),
        "the child's session keeps the hunks"
    );
    // The card the main conversation draws for the same call and change.
    let call = list(&live.state.collaboration[0], "tools")[0].clone();
    let mut main = State {
        width: 80,
        ..State::default()
    };
    main.begin_tool_turn();
    main.event(Event::ToolStarted {
        id: "edit-1".into(),
        name: "edit".into(),
        arguments: call.get("arguments").unwrap().clone(),
    });
    main.event(Event::ToolChanged {
        id: "edit-1".into(),
        change: changes.calls["edit-1"].clone(),
    });
    main.event(Event::ToolFinished {
        id: "edit-1".into(),
        name: "edit".into(),
        summary: value(&call, "text").into(),
        result: call.get("result").unwrap().clone(),
    });
    main.finish_tool_turn();
    for item in &mut main.items {
        if let Item::Tool { presentation, .. } = item {
            presentation.started = None;
            presentation.elapsed = None;
        }
    }
    let card = all(&main);
    assert!(shown.contains(&card), "{card}\n---\n{shown}");
    assert!(
        card.contains("second") && card.contains("changed"),
        "{card}"
    );
    let id = live.persistence.as_ref().unwrap().id();
    live.collaboration_observer.stop_all();
    drop(live);
    assert_eq!(fixture.finish().len(), 4);

    let mut resumed = app(
        &home,
        &project,
        "http://127.0.0.1:1/chat/completions".into(),
    );
    resumed.resume_session(&id);
    assert_eq!(chapters(&resumed.state.items), placed);
    // Read again from the child's saved session.
    assert_eq!(inside(&mut resumed), shown);
}

#[test]
fn inside_rows_are_reused_until_what_they_show_changes() {
    let mut state = State {
        width: 80,
        height: 24,
        collaboration: vec![record(1, "running")],
        ..State::default()
    };
    state.agents_panel = Some(Panel {
        inside: Some("a1".into()),
        ..Panel::default()
    });
    let draw = |state: &State| {
        let frame = view::frame(state, "fixture/model", "fixture");
        assert!(!frame.live.is_empty());
        state.agents_panel.as_ref().unwrap().rows.rows().unwrap()
    };
    let first = draw(&state);
    assert!(Arc::ptr_eq(&first, &draw(&state)));
    // Progress text is not drawn inside.
    set(
        &mut state.collaboration[0],
        "activity",
        Value::string("Reading src/search.rs"),
    );
    assert!(Arc::ptr_eq(&first, &draw(&state)));
    let mut tools = list(&state.collaboration[0], "tools")
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    tools.push(Value::object([
        ("id", Value::string("c2")),
        ("name", Value::string("read")),
        ("state", Value::string("running")),
        ("text", Value::string("")),
        (
            "arguments",
            Value::object([("path", Value::string("docs/usage/CLI.md"))]),
        ),
    ]));
    set(&mut state.collaboration[0], "tools", Value::Array(tools));
    let grown = draw(&state);
    assert!(!Arc::ptr_eq(&first, &grown));
    assert!(text(&grown).contains("docs/usage/CLI.md"));
    state.width = 60;
    let narrow = draw(&state);
    assert!(!Arc::ptr_eq(&grown, &narrow));
    assert!(Arc::ptr_eq(&narrow, &draw(&state)));
    // Freshly read changes drop the rows.
    state.agents_panel.as_ref().unwrap().rows.clear();
    assert!(!Arc::ptr_eq(&narrow, &draw(&state)));
}
