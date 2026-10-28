use super::*;
use crate::{
    todo::{Plan, Task},
    tui::{keys::Decoded, terminal::Key},
};

fn key(code: u16, modifiers: u8) -> Key {
    Key {
        code,
        modifiers,
        character: 0,
    }
}

fn plan() -> Plan {
    Plan {
        title: "CSV export".into(),
        revision: 2,
        tasks: vec![
            Task {
                id: "t1".into(),
                title: "Inspect report".into(),
                status: "done".into(),
                note: "Found the filter state".into(),
                agent: None,
            },
            Task {
                id: "t2".into(),
                title: "Export rows".into(),
                status: "in_progress".into(),
                note: "Use visible rows".into(),
                agent: Some("a1".into()),
            },
        ],
    }
}

#[test]
fn todo_filter_detail_and_nested_escape_preserve_chat_editor_at_small_widths() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.editor.insert("unsent draft");
    app.state.editor.cursor = 3;
    let chat = app.state.editor.clone();
    app.open_todo();
    let empty = view::composer_frame(&app.state, "fixture", "folder");
    assert!(
        empty
            .live
            .iter()
            .any(|line| line.plain().contains("No plan yet"))
    );
    app.state.todo_plan = Some(plan());
    app.state.width = 24;
    app.state.height = 18;
    app.input(Decoded::Text("t2".into())).unwrap();
    assert_eq!(
        app.state
            .todo_panel
            .as_ref()
            .unwrap()
            .filtered(app.state.todo_plan.as_ref().unwrap())
            .len(),
        1
    );
    app.input(Decoded::Key(key(13, 0))).unwrap();
    assert_eq!(
        app.state.todo_panel.as_ref().unwrap().detail_id.as_deref(),
        Some("t2")
    );
    let detail = view::composer_frame(&app.state, "fixture", "folder");
    assert!(
        detail
            .live
            .iter()
            .any(|line| line.plain().contains("Use visible rows"))
    );
    app.state.height = 12;
    let detail = view::composer_frame(&app.state, "fixture", "folder");
    assert!(detail.live.len() <= 8, "panel must respect two-thirds cap");
    app.input(Decoded::Key(key(27, 0))).unwrap();
    assert!(app.state.todo_panel.as_ref().unwrap().detail_id.is_none());
    app.input(Decoded::Key(key(27, 0))).unwrap();
    assert!(app.state.todo_panel.is_none());
    assert_eq!(app.state.editor, chat);
    assert!(fixture.finish().is_empty());
}

#[test]
fn todo_receipts_keep_each_revision_snapshot() {
    let mut state = State::default();
    let mut first = plan();
    first.revision = 1;
    first.tasks[1].status = "pending".into();
    let mut second = first.clone();
    second.revision = 2;
    second.tasks[1].status = "done".into();
    for (id, snapshot) in [("plan-1", first), ("plan-2", second)] {
        state.event(crate::events::Event::ToolStarted {
            id: id.into(),
            name: "todo_list".into(),
            arguments: Value::object([("action", Value::string("set"))]),
        });
        state.event(crate::events::Event::ToolFinished {
            id: id.into(),
            name: "todo_list".into(),
            summary: "complete".into(),
            result: snapshot.value(),
        });
    }
    let rows = state.rows();
    let first = rows[0].iter().map(|line| line.plain()).collect::<String>();
    let second = rows[1].iter().map(|line| line.plain()).collect::<String>();
    assert!(first.contains("revision 1") && first.contains("1 done"));
    assert!(second.contains("revision 2") && second.contains("2 done"));
}
