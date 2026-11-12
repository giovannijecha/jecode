use super::{Terminal, probe};
use crate::{
    ask::Request,
    events::Event,
    json,
    todo::{Plan, Task},
    tui::{
        line::Line,
        render::Renderer,
        state::State,
        text, theme,
        view::{self, Frame},
    },
};
use std::io::{self, Write};

fn paint(renderer: &mut Renderer, rows: Vec<Line>, width: usize, height: usize) {
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
    let output = renderer.paint(&frame, width, height);
    write!(io::stdout(), "{output}").unwrap();
    io::stdout().flush().unwrap();
    assert_eq!(probe().lines().nth(1).unwrap(), expected);
}

fn ask_frame(
    state: &State,
    renderer: &mut Renderer,
    width: usize,
    height: usize,
    expected: [&str; 3],
) {
    let frame = view::frame(state, "fixture/model", "fixture");
    let toggling = !expected[2].starts_with('›');
    for prefix in expected {
        assert!(
            frame.live.iter().any(|row| row.plain().starts_with(prefix)),
            "Missing {prefix}: {:?}",
            frame.live.iter().map(Line::plain).collect::<Vec<_>>()
        );
    }
    assert_eq!(
        frame
            .live
            .iter()
            .any(|row| row.plain().contains("Space toggle")),
        toggling,
        "Space types words when the cursor is on Other"
    );
    paint(renderer, frame.live, width, height);
}

fn multiple_answers(state: &mut State, renderer: &mut Renderer, width: usize, height: usize) {
    let request = state.question.clone().unwrap();
    let question = &request.questions[1];
    for columns in [40, width] {
        for light in [false, true] {
            theme::set_light(light);
            renderer.repaint();
            state.width = columns;
            let mut panel = crate::tui::ask::Panel::new(&request, None);
            panel.switch(&request, true);
            state.ask_panel = Some(panel);
            ask_frame(
                state,
                renderer,
                width,
                height,
                ["› ○ CSV ", "  ○ XLSX ", "  ○ Other "],
            );

            state.ask_panel.as_mut().unwrap().select(question);
            ask_frame(
                state,
                renderer,
                width,
                height,
                ["› ◉ CSV ", "  ○ XLSX ", "  ○ Other "],
            );

            let panel = state.ask_panel.as_mut().unwrap();
            panel.move_row(question, false);
            panel.select(question);
            ask_frame(
                state,
                renderer,
                width,
                height,
                ["  ◉ CSV ", "› ◉ XLSX ", "  ○ Other "],
            );

            state.ask_panel.as_mut().unwrap().select(question);
            ask_frame(
                state,
                renderer,
                width,
                height,
                ["  ◉ CSV ", "› ○ XLSX ", "  ○ Other "],
            );

            state
                .ask_panel
                .as_mut()
                .unwrap()
                .type_text(question, "Also JSON");
            ask_frame(
                state,
                renderer,
                width,
                height,
                ["  ◉ CSV ", "  ○ XLSX ", "› ◉ Other "],
            );
        }
    }
    theme::set_light(false);
    renderer.repaint();
    state.width = width;
}

pub(super) fn console() {
    let terminal = Terminal::open().unwrap();
    let (width, height) = terminal.size;
    let mut renderer = Renderer::new();
    let mut state = State {
        width,
        height,
        ..State::default()
    };
    state.editor.insert("main draft survives");
    state.editor.cursor = 4;
    let draft = state.editor.clone();
    let arguments = json::parse(
        r#"{"questions":[{"id":"scope","title":"Export scope",
        "question":"Which report rows should the CSV include?",
        "context":"The current report is filtered by date.","options":[
        {"id":"visible","label":"Visible rows","description":"Keep filters","recommended":true},
        {"id":"all","label":"All rows","description":"Ignore filters"}]},
        {"id":"formats","title":"Formats","question":"Which formats?","multiple":true,
        "options":[{"id":"csv","label":"CSV"},{"id":"xlsx","label":"XLSX"}]}]}"#,
    )
    .unwrap();
    let request = Request::parse("ask-1", &arguments).unwrap();
    state.event(Event::ToolStarted {
        id: request.id.clone(),
        name: "ask_user".into(),
        arguments: request.arguments(),
    });
    state.event(Event::UserQuestion { request });
    let ask = view::frame(&state, "fixture/model", "fixture");
    let shown: Vec<String> = ask.live.iter().map(Line::plain).collect();
    for expected in [
        "?  Waiting for you",
        "  Export scope   Formats",
        "  Which report rows should the CSV include?",
        "› ○ Visible rows",
        "  ○ All rows",
        "  ○ Other",
        "Enter next",
    ] {
        assert!(
            shown.iter().any(|row| row.contains(expected)),
            "{expected}\n{shown:#?}"
        );
    }
    assert!(
        shown
            .iter()
            .any(|row| row.trim_end().ends_with("recommended"))
    );
    assert!(!shown.iter().any(|row| row.contains("ASK USER")));
    assert_eq!(state.editor, draft);
    paint(&mut renderer, ask.live, width, height);
    multiple_answers(&mut state, &mut renderer, width, height);

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
    paint(&mut renderer, created, width, height);

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
        row.contains("Check CSV encoding") && row.contains("AGENTS  #1") && row.contains("review")
    }));
    assert_eq!(state.editor, draft);
    paint(&mut renderer, todo.live, width, height);
    drop(terminal);
}
