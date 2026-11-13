use super::{Terminal, probe};
use crate::{
    ask::Request,
    events::Event,
    json::{self, Value},
    todo::{Phase, Plan, Task, owner},
    tui::{
        line::Line,
        render::Renderer,
        state::State,
        text, theme,
        view::{self, Frame},
    },
};
use std::io::{self, Write};

pub(super) fn paint(
    renderer: &mut Renderer,
    rows: Vec<Line>,
    width: usize,
    height: usize,
) -> String {
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
    output
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
    for columns in [40, 80] {
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
                ["  ◉ CSV ", "  ○ XLSX ", "› ◉ Other  Also JSON"],
            );
        }
    }
    theme::set_light(false);
    renderer.repaint();
    state.width = width;
}

/// A task as (title, status, collaborator); collaborators are `a1`, `a2`, ...
type Row = (&'static str, &'static str, Option<&'static str>);

/// Successive `/todo` states: every phase, then completion reopened and given,
/// a reviewed task completed, retitled and newly cancelled tasks, then fewer
/// rows, all done. The collaborator lifecycles follow each task list.
const STAGES: [(&[Row], &[&str]); 3] = [
    (
        &[
            ("Inspect the report", "done", None),
            ("Write the exporter", "in_progress", None),
            ("Cover edge cases", "in_progress", Some("a1")),
            ("Benchmark large files", "pending", Some("a2")),
            ("Ask about encoding", "waiting", None),
            ("Support XML", "cancelled", None),
            ("Update the docs", "pending", None),
        ],
        &["returned", "failed"],
    ),
    (
        &[
            ("Inspect the report", "in_progress", None),
            ("Write the exporter", "done", None),
            ("Cover edge cases", "done", Some("a1")),
            ("Benchmark large files again", "in_progress", Some("a2")),
            ("Ask", "waiting", None),
            ("Support XML", "cancelled", None),
            ("Update the docs", "cancelled", None),
        ],
        &["returned", "running"],
    ),
    (
        &[
            ("Inspect the report", "done", None),
            ("Write the exporter", "done", None),
            ("Cover edge cases", "done", Some("a1")),
            ("Benchmark large files again", "done", Some("a2")),
            ("Support XML", "cancelled", None),
        ],
        &["returned", "returned"],
    ),
];

fn stage(state: &mut State, tasks: &[Row], lifecycles: &[&str]) {
    state.todo_plan = Some(Plan {
        title: "CSV export".into(),
        revision: 1,
        tasks: tasks
            .iter()
            .enumerate()
            .map(|(index, (title, status, agent))| Task {
                id: format!("t{index}"),
                title: (*title).into(),
                status: (*status).into(),
                note: String::new(),
                agent: agent.map(Into::into),
            })
            .collect(),
    });
    state.collaboration = lifecycles
        .iter()
        .enumerate()
        .map(|(index, lifecycle)| {
            Value::object([
                ("id", Value::string(format!("a{}", index + 1))),
                ("number", Value::number(index + 1)),
                ("state", Value::string(*lifecycle)),
            ])
        })
        .collect();
}

/// Every task row is in the sheet with `◉ ` only when done and `○ ` otherwise,
/// titles start on one column and state words share theirs; the console then
/// reads back exactly this frame, so earlier marks and titles left no cells.
fn todo_frame(state: &State, renderer: &mut Renderer, width: usize, height: usize) {
    let plan = state.todo_plan.as_ref().unwrap();
    let records = &state.collaboration;
    let narrow = state.width < 64;
    let frame = view::frame(state, "fixture/model", "fixture");
    let shown: Vec<String> = frame.live.iter().map(Line::plain).collect();
    assert!(!shown.iter().any(|row| row.contains(['☐', '☑'])));
    let rows = view::todo::rows(plan, records, state.width);
    let (mut number, mut words) = (0, Vec::new());
    for (task, row) in plan.tasks.iter().zip(rows.iter().map(Line::plain)) {
        assert!(
            shown.iter().any(|line| line.starts_with(row.trim_end())),
            "Missing {row:?}: {shown:#?}"
        );
        let phase = Phase::of(task, owner(task, records));
        let lead = if narrow {
            "  ".to_owned()
        } else if phase == Phase::Cancelled {
            " ".repeat(6)
        } else {
            number += 1;
            format!("  {number:>2}  ")
        };
        let mark = if phase == Phase::Done { '◉' } else { '○' };
        let head: String = task.title.chars().take(6).collect();
        let expected = format!("{lead}{mark} {head}");
        assert!(row.starts_with(&expected), "{row:?} lacks {expected:?}");
        let word = phase.word(narrow);
        if !word.is_empty() {
            let before = row.trim_end().strip_suffix(word);
            words.push(text::cells(before.unwrap_or_else(|| panic!("{row:?}"))));
        }
    }
    assert!(words.windows(2).all(|pair| pair[0] == pair[1]), "{rows:?}");
    paint(renderer, frame.live, width, height);
}

/// `/todo` at 40 and 80 columns in both palettes, through successive states
/// and the empty states, painted by one renderer that only repaints in full
/// when the palette changes.
fn todo_marks(state: &mut State, renderer: &mut Renderer, width: usize, height: usize) {
    state.todo_panel = Some(crate::tui::todo::Panel::default());
    for columns in [40, 80] {
        for light in [false, true] {
            theme::set_light(light);
            renderer.repaint();
            state.width = columns;
            for (tasks, lifecycles) in STAGES {
                stage(state, tasks, lifecycles);
                todo_frame(state, renderer, width, height);
            }
            for (plan, message) in [
                (Some(Plan::empty()), "This plan has no tasks."),
                (None, "No plan yet."),
            ] {
                state.todo_plan = plan;
                let frame = view::frame(state, "fixture/model", "fixture");
                let shown: Vec<String> = frame.live.iter().map(Line::plain).collect();
                assert!(shown.iter().any(|row| row.contains(message)), "{shown:#?}");
                assert!(!shown.iter().any(|row| row.contains(['◉', '○'])));
                paint(renderer, frame.live, width, height);
            }
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
    todo_marks(&mut state, &mut renderer, width, height);
    super::mark_tests::console(&mut renderer, width, height);
    drop(terminal);
}
