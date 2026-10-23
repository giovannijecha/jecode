use super::*;
use crate::{events::Event, tui::state::Item};

fn key(app: &mut App, code: u16, modifiers: u8) {
    assert!(
        !app.input(Decoded::Key(terminal::Key {
            code,
            modifiers,
            character: 0,
        }))
        .unwrap()
    );
}

fn tool(app: &mut App, id: &str, output: &str) {
    app.state.event(Event::ToolStarted {
        id: id.into(),
        name: "bash".into(),
        arguments: Value::object([("command", Value::string(id))]),
    });
    app.state.event(Event::ToolFinished {
        id: id.into(),
        name: "bash".into(),
        summary: "exit 0".into(),
        result: Value::object([
            ("exit_code", Value::number(0)),
            ("stdout", Value::string(output)),
        ]),
    });
}

#[test]
fn tool_toggle_preserves_the_draft_and_source_and_returns_to_editing() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.begin_tool_turn();
    tool(&mut app, "first", "one\n\nthree\nfour\nfive\n");
    tool(&mut app, "second", "second output");
    app.state.message(Kind::Assistant, "Between groups.");
    tool(&mut app, "third", "third output");
    app.state.close_tools();
    app.state.finish_tool_turn();
    app.state.editor.insert("kept draft");
    let caret = app.state.editor.cursor;
    let generation = app.state.generation;
    let compact = app.state.rows()[0]
        .iter()
        .map(|row| row.plain())
        .collect::<Vec<_>>();
    assert!(compact.iter().any(|row| row.contains("five")));
    assert!(!compact.iter().any(|row| row.trim() == "one"));

    key(&mut app, 79, 4); // Ctrl+O expands every tool in the completed turn.
    let rows: Vec<_> = app
        .state
        .rows()
        .into_iter()
        .flatten()
        .map(|row| row.plain())
        .collect();
    assert!(rows.iter().any(|row| row == "    one"));
    assert!(rows.iter().any(|row| row == "    "));
    assert!(rows.iter().any(|row| row == "    third output"));
    assert_eq!(app.state.generation, generation);
    assert_eq!(app.state.editor.text, "kept draft");
    assert_eq!(app.state.editor.cursor, caret);
    assert!(app.worker.is_none() && app.state.queue.messages().is_empty());
    let Item::Tool {
        result: Some(result),
        ..
    } = &app.state.items[0]
    else {
        panic!("missing tool")
    };
    assert_eq!(
        result.get("stdout").and_then(Value::as_str),
        Some("one\n\nthree\nfour\nfive\n")
    );

    key(&mut app, 79, 4);
    assert!(!app.state.tool_details.expanded);
    assert_eq!(app.state.editor.text, "kept draft");
    key(&mut app, 37, 0);
    assert_eq!(app.state.editor.cursor, caret - 1);
    app.input(Decoded::Text("!".into())).unwrap();
    assert_eq!(app.state.editor.text, "kept draf!t");
    app.state.editor.replace("/he".into());
    app.edited();
    key(&mut app, 9, 0);
    assert_eq!(app.state.editor.text, "/help");
    assert!(fixture.finish().is_empty());
}

#[test]
fn control_o_during_a_running_turn_does_not_cancel_it_or_lose_its_draft() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Finished.", vec![]))]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "A fixture turn.");
    tool(&mut app, "fixture tool", "retained result");
    app.state.editor.insert("next draft");
    key(&mut app, 79, 4);
    assert!(!app.state.tool_details.expanded);
    assert!(app.worker.is_some());
    assert!(!app.state.activity.as_ref().unwrap().stopping);
    assert_eq!(app.state.editor.text, "next draft");
    finish(&mut app);
    assert_eq!(app.state.status, "Ready");
    assert_eq!(app.state.editor.text, "next draft");
    key(&mut app, 79, 4);
    assert!(app.state.tool_details.expanded);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn queued_exit_waits_for_completion_and_quit_alias_uses_the_same_path() {
    for command in ["/exit", "/QuIt"] {
        let directory = Directory::new();
        let fixture = HttpFixture::new(vec![(200, completion("Finished.", vec![]))]);
        let mut app = app(&directory, &fixture);
        submit(&mut app, "Finish the request");
        submit(&mut app, command);
        assert!(!app.exit_requested);
        assert_eq!(app.state.queue.messages()[0].text, command);
        finish(&mut app);
        assert!(app.exit_requested);
        assert!(rows(&app).iter().any(|row| row == "assistant: Finished."));
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[test]
fn queued_exit_pauses_when_the_active_turn_fails() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(
        401,
        Value::object([(
            "error",
            Value::object([("message", Value::string("Fixture rejection"))]),
        )]),
    )]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Fail the request");
    submit(&mut app, "/exit");
    finish(&mut app);
    assert!(!app.exit_requested);
    assert_eq!(app.state.queue.paused[0].text, "/exit");
    assert!(app.state.queue.messages().is_empty());
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn queued_init_starts_after_the_active_turn_and_keeps_its_visible_name() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("First complete.", vec![])),
        (200, completion("Instructions inspected.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "First task");
    submit(&mut app, "/init");
    assert_eq!(app.state.queue.messages()[0].text, "/init");
    finish(&mut app);
    assert!(rows(&app).iter().any(|row| row == "user: /init"));
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(last_content(&requests[0]), "First task");
    assert!(last_content(&requests[1]).starts_with("/init\n\n"));
}

#[test]
fn init_stays_retryable_when_project_instructions_cannot_be_read() {
    let directory = Directory::new();
    fs::write(directory.path().join("AGENTS.md"), [0xff, 0xfe]).unwrap();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "/init");
    assert!(app.worker.is_none());
    assert_eq!(app.state.queue.paused[0].text, "/init");
    assert!(app.state.notice.as_ref().unwrap().text.contains("UTF-8"));
    assert!(fixture.finish().is_empty());
}
