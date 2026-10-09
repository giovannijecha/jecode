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
fn tool_inspection_preserves_the_draft_and_source_and_returns_to_editing() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    tool(&mut app, "first", "one\n\nthree\nfour\nfive\n");
    tool(&mut app, "second", "second output");
    app.state.message(Kind::Assistant, "Between groups.");
    tool(&mut app, "third", "third output");
    app.state.close_tools();
    app.state.editor.insert("kept draft");
    let caret = app.state.editor.cursor;
    let generation = app.state.generation;
    assert!(
        !app.state
            .rows()
            .iter()
            .flatten()
            .any(|row| row.plain().contains("five"))
    );

    key(&mut app, 84, 1); // Alt+T opens the newest tool.
    assert_eq!(app.state.tool_focus, Some(3));
    key(&mut app, 38, 0); // Text between groups is skipped.
    assert_eq!(app.state.tool_focus, Some(1));
    key(&mut app, 38, 0);
    assert_eq!(app.state.tool_focus, Some(0));
    key(&mut app, 13, 0);
    let rows: Vec<_> = app
        .state
        .rows()
        .into_iter()
        .flatten()
        .map(|row| row.plain())
        .collect();
    assert!(rows.iter().any(|row| row == "│    five"));
    assert!(rows.iter().any(|row| row == "│    "));
    assert_eq!(app.state.generation, generation);
    assert_eq!(app.state.editor.text, "kept draft");
    assert_eq!(app.state.editor.cursor, caret);
    assert!(app.worker.is_none() && app.state.queue.messages.is_empty());
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

    key(&mut app, 9, 0); // Tab returns without sending or completing the draft.
    assert!(app.state.tool_focus.is_none());
    key(&mut app, 84, 1);
    key(&mut app, 37, 0); // Editing keys return to the composer.
    assert!(app.state.tool_focus.is_none());
    assert_eq!(app.state.editor.cursor, caret - 1);
    key(&mut app, 84, 1);
    app.input(Decoded::Text("!".into())).unwrap();
    assert!(app.state.tool_focus.is_none());
    assert_eq!(app.state.editor.text, "kept draf!t");
    app.state.editor.replace("/he".into());
    app.edited();
    key(&mut app, 84, 1);
    key(&mut app, 27, 0);
    key(&mut app, 9, 0);
    assert_eq!(app.state.editor.text, "/help");
    assert!(fixture.finish().is_empty());
}

#[test]
fn leaving_tool_inspection_does_not_cancel_a_running_turn_or_lose_its_draft() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Finished.", vec![]))]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "A fixture turn.");
    tool(&mut app, "fixture tool", "retained result");
    app.state.editor.insert("next draft");
    key(&mut app, 84, 1);
    key(&mut app, 27, 0);
    assert!(app.state.tool_focus.is_none());
    assert!(app.worker.is_some());
    assert!(!app.state.activity.as_ref().unwrap().stopping);
    assert_eq!(app.state.editor.text, "next draft");
    finish(&mut app);
    assert_eq!(app.state.status, "Ready");
    assert_eq!(app.state.editor.text, "next draft");
    assert_eq!(fixture.finish().len(), 1);
}
