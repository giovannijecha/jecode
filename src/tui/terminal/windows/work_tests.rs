use super::{Terminal, probe};
use crate::{
    ask::{Choice, Question, Request},
    events::Event,
    todo::{Plan, Task},
    tui::{
        line::Line,
        render::Renderer,
        state::State,
        text,
        view::{self, Frame},
    },
};
use std::{
    io::{self, Write},
    path::Path,
};

fn paint(rows: Vec<Line>, width: usize, height: usize) {
    assert!(rows.len() <= height);
    let expected: String = (0..height)
        .map(|index| {
            let row = rows.get(index).map_or_else(String::new, Line::plain);
            format!(
                "{row}{}",
                " ".repeat(width.saturating_sub(text::cells(&row)))
            )
        })
        .collect();
    let frame = Frame {
        header: vec![],
        history: vec![],
        live: rows,
        cursor: None,
        composer: height - 1,
    };
    let output = Renderer::new().paint(&frame, width, height);
    write!(io::stdout(), "{output}").unwrap();
    io::stdout().flush().unwrap();
    assert_eq!(probe().lines().nth(1).unwrap(), expected);
}

pub(super) fn console(bash: &Path) {
    let terminal = Terminal::open(bash).unwrap();
    let (width, height) = terminal.size;
    let mut state = State {
        width,
        height,
        ..State::default()
    };
    state.editor.insert("main draft survives");
    state.editor.cursor = 4;
    let draft = state.editor.clone();
    let request = Request {
        id: "ask-1".into(),
        question: Question {
            title: "Export scope".into(),
            question: "Which report rows should the CSV include?".into(),
            context: "The current report is filtered by date.".into(),
            options: vec![
                Choice {
                    id: "visible".into(),
                    label: "Visible rows".into(),
                    description: "Keep filters".into(),
                    recommended: true,
                },
                Choice {
                    id: "all".into(),
                    label: "All rows".into(),
                    description: "Ignore filters".into(),
                    recommended: false,
                },
            ],
        },
    };
    state.event(Event::ToolStarted {
        id: request.id.clone(),
        name: "ask_user".into(),
        arguments: request.question.value(),
    });
    state.event(Event::UserQuestion { request });
    let ask = view::frame(&state, "fixture/model", "fixture");
    assert!(
        ask.live
            .iter()
            .any(|row| row.plain().contains("Waiting for you"))
    );
    assert!(ask.live.iter().any(|row| row.plain().contains("Ctrl+D")));
    assert_eq!(state.editor, draft);
    paint(ask.live, width, height);

    state.ask_panel = None;
    // Only the call creating the plan has a row; the next set has none.
    let mut plan = None;
    for (id, status) in [("todo-1", "pending"), ("todo-2", "in_progress")] {
        let arguments = crate::json::Value::object([
            ("action", crate::json::Value::string("set")),
            ("title", crate::json::Value::string("CSV export")),
            (
                "tasks",
                crate::json::Value::Array(vec![crate::json::Value::object([
                    ("id", crate::json::Value::string("t1")),
                    ("title", crate::json::Value::string("Choose exported rows")),
                    ("status", crate::json::Value::string(status)),
                ])]),
            ),
        ]);
        let next = Plan::set(&arguments, plan.as_ref()).unwrap();
        state.event(Event::ToolStarted {
            id: id.into(),
            name: "todo_list".into(),
            arguments,
        });
        state.event(Event::ToolFinished {
            id: id.into(),
            name: "todo_list".into(),
            summary: crate::events::tool_summary("todo_list", &next.value()),
            result: next.value(),
        });
        plan = Some(next);
    }
    let created: Vec<Line> = state
        .rows()
        .concat()
        .into_iter()
        .filter(|row| row.plain().contains("TODO"))
        .collect();
    assert_eq!(created.len(), 1);
    assert!(created[0].plain().contains("CSV export  1 task"));
    paint(created, width, height);

    state.collaboration = vec![crate::json::Value::object([
        ("id", crate::json::Value::string("a1")),
        ("number", crate::json::Value::number(1)),
        ("state", crate::json::Value::string("returned")),
    ])];
    state.todo_plan = Some(Plan {
        title: "CSV export".into(),
        revision: 1,
        tasks: vec![
            Task {
                id: "t1".into(),
                title: "Choose exported rows".into(),
                status: "waiting".into(),
                note: "Awaiting answer".into(),
                agent: None,
            },
            Task {
                id: "t2".into(),
                title: "Check CSV encoding".into(),
                status: "in_progress".into(),
                note: "Child result needs main review".into(),
                agent: Some("a1".into()),
            },
        ],
    });
    state.todo_panel = Some(crate::tui::todo::Panel::default());
    let todo = view::frame(&state, "fixture/model", "fixture");
    assert!(todo.live.iter().any(|row| row.plain().contains("Todo")));
    assert!(
        todo.live
            .iter()
            .any(|row| row.plain().contains("Choose exported rows"))
    );
    assert!(todo.live.iter().any(|row| {
        let row = row.plain();
        row.contains("Check CSV encoding") && row.contains("AGENT 1") && row.contains("review")
    }));
    assert_eq!(state.editor, draft);
    paint(todo.live, width, height);
    drop(terminal);
}
