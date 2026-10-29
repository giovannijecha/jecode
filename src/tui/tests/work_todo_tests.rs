use super::*;
use crate::{
    events::Event,
    todo::{Plan, Task},
    tui::{keys::Decoded, line::Line, terminal::Key},
};

fn key(code: u16) -> Key {
    Key {
        code,
        modifiers: 0,
        character: 0,
    }
}

/// A set call's arguments; each task is `(id, status)` titled `Task <id>`.
fn set(title: &str, tasks: &[(&str, &str)]) -> Value {
    Value::object([
        ("action", Value::string("set")),
        ("title", Value::string(title)),
        (
            "tasks",
            Value::Array(
                tasks
                    .iter()
                    .map(|(id, status)| {
                        Value::object([
                            ("id", Value::string(*id)),
                            ("title", Value::string(format!("Task {id}"))),
                            ("status", Value::string(*status)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn get() -> Value {
    Value::object([("action", Value::string("get"))])
}

/// Runs a todo_list call through the transcript as the agent would answer it.
fn call(state: &mut State, plan: &mut Option<Plan>, id: &str, arguments: Value) {
    state.event(Event::ToolStarted {
        id: id.into(),
        name: "todo_list".into(),
        arguments: arguments.clone(),
    });
    let result = if arguments.get("action").and_then(Value::as_str) == Some("set") {
        match Plan::set(&arguments, plan.as_ref()) {
            Ok(next) => {
                *plan = Some(next.clone());
                next.value()
            }
            Err(error) => Value::object([("error", Value::string(&error))]),
        }
    } else {
        plan.clone().unwrap_or_else(Plan::empty).value()
    };
    state.event(Event::ToolFinished {
        id: id.into(),
        name: "todo_list".into(),
        summary: crate::events::tool_summary("todo_list", &result),
        result,
    });
}

fn todo_rows(state: &State) -> Vec<String> {
    state
        .rows()
        .into_iter()
        .zip(&state.items)
        .filter(|(_, item)| matches!(item, state::Item::Tool { name, .. } if name == "todo_list"))
        .map(|(rows, _)| {
            let rows = rows.iter().map(Line::plain).filter(|row| !row.is_empty());
            rows.collect::<Vec<_>>().join("\n")
        })
        .collect()
}

#[test]
fn only_the_call_creating_a_plan_has_a_conversation_row() {
    let mut state = State::default();
    let mut plan = None;
    call(&mut state, &mut plan, "read-none", get());
    let open = [("t1", "in_progress"), ("t2", "pending"), ("t3", "pending")];
    call(&mut state, &mut plan, "create", set("CSV export", &open));
    call(&mut state, &mut plan, "read", get());
    let step = [("t1", "done"), ("t2", "in_progress"), ("t3", "pending")];
    call(&mut state, &mut plan, "step", set("CSV export", &step));
    let replan = [("t1", "done"), ("t2", "in_progress"), ("t3", "cancelled")];
    call(
        &mut state,
        &mut plan,
        "replan",
        set("CSV export via menu", &replan),
    );
    call(
        &mut state,
        &mut plan,
        "invalid",
        set("CSV export", &[("t1", "bad")]),
    );
    let end = [("t1", "done"), ("t2", "done"), ("t3", "cancelled")];
    call(
        &mut state,
        &mut plan,
        "end",
        set("CSV export via menu", &end),
    );
    call(&mut state, &mut plan, "after-end", get());
    let next = [("n1", "pending"), ("n2", "pending")];
    call(&mut state, &mut plan, "next", set("Release notes", &next));
    assert_eq!(
        todo_rows(&state),
        [
            " TODO  CSV export  3 tasks",
            " TODO  Release notes  2 tasks"
        ]
    );
    assert_eq!(plan.unwrap().revision, 5);
    // A row stays as written while the plan moves on, at any width.
    state.width = 24;
    assert_eq!(todo_rows(&state)[0], " TODO  CSV exp…  3 tasks");
}

#[test]
fn a_failed_creating_call_keeps_its_row_and_the_next_set_creates_again() {
    let mut state = State::default();
    let mut plan = None;
    call(
        &mut state,
        &mut plan,
        "bad",
        set("Export", &[("t1", "unknown")]),
    );
    call(
        &mut state,
        &mut plan,
        "good",
        set("Export", &[("t1", "pending")]),
    );
    let rows = todo_rows(&state);
    assert_eq!(rows.len(), 2);
    assert!(rows[0].starts_with(" TODO  Export  1 task\n  └ "));
    assert!(rows[0].contains("Invalid task status"));
    assert_eq!(rows[1], " TODO  Export  1 task");
}

#[test]
fn hidden_calls_neither_split_chapters_nor_offer_ctrl_o() {
    let mut state = State::default();
    let mut plan = None;
    call(
        &mut state,
        &mut plan,
        "create",
        set("Map", &[("t1", "pending")]),
    );
    state.begin_tool_turn();
    for (id, path) in [("r1", "a.rs"), ("r2", "b.rs")] {
        state.event(Event::ToolStarted {
            id: id.into(),
            name: "read".into(),
            arguments: Value::object([
                ("path", Value::string(path)),
                ("work", Value::string("Map the flow")),
            ]),
        });
        state.event(Event::ToolFinished {
            id: id.into(),
            name: "read".into(),
            summary: "1 line".into(),
            result: Value::object([("content", Value::string("fn main() {}"))]),
        });
        call(&mut state, &mut plan, &format!("{id}-step"), get());
    }
    state.finish_tool_turn();
    assert_eq!(state.items.len(), 3);
    let chapter = state.rows()[1..].concat();
    assert_eq!(chapter[1].plain(), " WORK  Map the flow  2 calls");

    state.begin_tool_turn();
    call(&mut state, &mut plan, "done", set("Map", &[("t1", "done")]));
    call(&mut state, &mut plan, "read", get());
    state.finish_tool_turn();
    assert_eq!(state.items.len(), 3);
    assert!(!state.tool_details_available());
    assert!(state.toggle_tools().is_none());
}

#[test]
fn creating_rows_and_the_latest_plan_survive_resume() {
    let directory = Directory::new();
    let home = Directory::new();
    let open = set("CSV export", &[("t1", "in_progress"), ("t2", "pending")]);
    let step = set("CSV export", &[("t1", "done"), ("t2", "in_progress")]);
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    tool_call("todo-1", "todo_list", open),
                    tool_call("todo-2", "todo_list", get()),
                ],
            ),
        ),
        (
            200,
            completion("", vec![tool_call("todo-3", "todo_list", step)]),
        ),
        (200, completion("Export planned.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let mut app = App::new(agent, config(&directory), None);
    submit(&mut app, "Add CSV export");
    finish(&mut app);
    assert_eq!(todo_rows(&app.state), [" TODO  CSV export  2 tasks"]);
    assert_eq!(app.state.todo_plan.as_ref().unwrap().revision, 2);
    let id = app.persistence.as_ref().unwrap().id();
    drop(app);
    assert_eq!(fixture.finish().len(), 3);

    let next = HttpFixture::new(vec![]);
    let mut agent = Agent::new(
        OpenRouter::fixture(next.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let mut resumed = App::new(agent, config(&directory), None);
    resumed.resume_session(&id);
    assert_eq!(todo_rows(&resumed.state), [" TODO  CSV export  2 tasks"]);
    let plan = resumed.state.todo_plan.as_ref().unwrap();
    assert_eq!((plan.revision, plan.tasks[0].status.as_str()), (2, "done"));
    // A later set on the open plan adds no row after resume either.
    let mut plan = Some(plan.clone());
    let replan = set("CSV export", &[("t1", "done"), ("t2", "done")]);
    call(&mut resumed.state, &mut plan, "todo-4", replan);
    assert_eq!(todo_rows(&resumed.state).len(), 1);
    assert!(next.finish().is_empty());
}

fn long_plan() -> Plan {
    let status = |index: usize| match index {
        0..4 => "done",
        4 | 5 => "in_progress",
        _ => "pending",
    };
    Plan {
        title: "Session storage migration".into(),
        revision: 3,
        tasks: (0..14)
            .map(|index| Task {
                id: format!("t{index}"),
                title: format!("Task {index}"),
                status: status(index).into(),
                note: String::new(),
                agent: None,
            })
            .collect(),
    }
}

fn delegated_plan() -> (Plan, Vec<Value>) {
    let task = |id: &str, title: &str, status: &str, agent: Option<&str>| Task {
        id: id.into(),
        title: title.into(),
        status: status.into(),
        note: "for the model".into(),
        agent: agent.map(Into::into),
    };
    let plan = Plan {
        title: "CSV export".into(),
        revision: 4,
        tasks: vec![
            task("t1", "Inspect the report", "done", None),
            task("t2", "Write the exporter", "in_progress", None),
            task("t3", "Cover edge cases", "in_progress", Some("a1")),
            task("t4", "Benchmark large files", "pending", Some("a2")),
            task("t5", "Ask about encoding", "waiting", None),
            task("t6", "Support XML", "cancelled", None),
            task("t7", "Update the docs", "pending", None),
        ],
    };
    let record = |id: &str, number: usize, state: &str| {
        Value::object([
            ("id", Value::string(id)),
            ("number", Value::number(number)),
            ("state", Value::string(state)),
        ])
    };
    let records = vec![record("a1", 1, "returned"), record("a2", 2, "failed")];
    (plan, records)
}

#[test]
fn menu_rows_align_owner_and_state_columns_and_narrow_drops_numbers() {
    let (plan, records) = delegated_plan();
    let rows = |columns| {
        view::todo::rows(&plan, &records, columns)
            .iter()
            .map(Line::plain)
            .collect::<Vec<_>>()
    };
    let pad = |row: &str| format!("{row:<78}");
    assert_eq!(
        rows(80),
        [
            "   1  ☑ Inspect the report                                     done",
            "   2  ☐ Write the exporter                                     in progress",
            "   3  ☐ Cover edge cases                             AGENT 1   awaiting review",
            "   4  ☐ Benchmark large files                        AGENT 2   failed",
            "   5  ☐ Ask about encoding                                     waiting",
            "      ☐ Support XML                                            cancelled",
            "   6  ☐ Update the docs",
        ]
        .map(pad)
    );
    let pad = |row: &str| format!("{row:<46}");
    assert_eq!(
        rows(48),
        [
            "  ☑ Inspect the report             done",
            "  ☐ Write the exporter             in progress",
            "  ☐ Cover edge cases     AGENT 1   review",
            "  ☐ Benchmark large f…   AGENT 2   failed",
            "  ☐ Ask about encoding             waiting",
            "  ☐ Support XML                    cancelled",
            "  ☐ Update the docs",
        ]
        .map(pad)
    );
    // Marks, titles and words carry the state colours.
    let lines = view::todo::rows(&plan, &records, 80);
    let style = |row: usize, text: &str| {
        let lines = &lines[row];
        lines
            .spans
            .iter()
            .find(|span| span.text.contains(text))
            .unwrap()
            .style
    };
    use crate::tui::theme::{ACCENT, BAD, DIM, GOOD, MUTED, STRONG};
    assert_eq!(style(0, "☑"), GOOD);
    assert_eq!(style(0, "Inspect"), MUTED);
    assert_eq!(style(1, "☐"), ACCENT.bold());
    assert_eq!(style(1, "Write"), STRONG.bold());
    assert_eq!(style(2, "awaiting review"), MUTED);
    assert_eq!(style(3, "☐"), BAD.bold());
    assert_eq!(style(3, "failed"), BAD);
    assert_eq!(style(5, "Support"), DIM.strike());
    assert_eq!(style(5, "cancelled"), DIM);
    assert_eq!(style(6, "☐"), MUTED);
}

#[test]
fn the_menu_rule_carries_the_plan_title_and_the_progress_strip() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let (plan, records) = delegated_plan();
    app.state.todo_plan = Some(plan);
    app.state.collaboration = records;
    app.open_todo();
    let rule = |app: &App| {
        let frame = view::composer_frame(&app.state, "fixture", "folder");
        let rows = frame.live.iter().map(Line::plain);
        rows.into_iter().find(|row| row.contains("Todo")).unwrap()
    };
    app.state.width = 80;
    let wide = rule(&app);
    assert!(wide.contains("Todo · CSV export"), "{wide}");
    assert!(wide.ends_with(" ▰▰▰▰▰▱ 1 of 6 done ─"), "{wide}");
    assert!(!wide.contains("↑↓"), "nothing to scroll");
    app.state.width = 30;
    let narrow = rule(&app);
    assert!(narrow.contains("Todo · CSV export"), "{narrow}");
    assert!(!narrow.contains("done"), "{narrow}");
    assert!(fixture.finish().is_empty());
}

/// The task titles in the open menu, top to bottom.
fn shown(app: &App) -> Vec<String> {
    view::composer_frame(&app.state, "fixture", "folder")
        .live
        .iter()
        .map(Line::plain)
        .filter_map(|row| {
            let start = row.find("Task ")?;
            Some(
                row[start..]
                    .split_whitespace()
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        })
        .collect()
}

#[test]
fn the_menu_opens_above_the_first_unfinished_task_and_scrolls_without_a_cursor() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.width = 80;
    app.state.height = 32;
    app.state.editor.insert("unsent draft");
    app.state.editor.cursor = 3;
    let chat = app.state.editor.clone();
    app.open_todo();
    app.state.todo_plan = Some(long_plan());
    let room = view::todo_capacity(&app.state);
    assert!(room < 14, "the long plan must overflow");
    let rows = shown(&app);
    assert_eq!(rows.len(), room);
    assert_eq!(rows[0], "Task 3", "one task above the first unfinished");
    let frame = view::composer_frame(&app.state, "fixture", "folder");
    let thumb = frame
        .live
        .iter()
        .filter(|row| row.plain().ends_with('┃'))
        .count();
    assert!(thumb > 0 && thumb < room);
    let mut tasks = frame
        .live
        .iter()
        .filter(|row| row.plain().contains("Task "));
    assert!(!tasks.any(view::focused), "no cursor");
    assert!(frame.live.iter().any(|row| {
        let row = row.plain();
        row.contains("↑↓") && row.contains("scroll") && row.contains("Esc") && row.contains("close")
    }));

    app.input(Decoded::Key(key(38))).unwrap();
    assert_eq!(shown(&app)[0], "Task 2");
    app.input(Decoded::Text("typed text".into())).unwrap();
    assert_eq!(app.state.editor, chat);
    app.input(Decoded::Key(key(35))).unwrap();
    assert_eq!(shown(&app).last().unwrap(), "Task 13");
    app.input(Decoded::Key(key(40))).unwrap();
    assert_eq!(shown(&app).last().unwrap(), "Task 13");
    app.input(Decoded::Key(key(36))).unwrap();
    assert_eq!(shown(&app)[0], "Task 0");
    app.input(Decoded::Scroll(2)).unwrap();
    assert_eq!(shown(&app)[0], "Task 2");
    app.input(Decoded::Key(key(27))).unwrap();
    assert!(app.state.todo_panel.is_none());
    assert_eq!(app.state.editor, chat);
    assert!(fixture.finish().is_empty());
}

#[test]
fn without_a_plan_the_menu_shows_one_dim_line_and_only_escape() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    for (width, height) in [(80, 24), (40, 16), (120, 40), (24, 12)] {
        app.state.width = width;
        app.state.height = height;
        submit(&mut app, "/todo");
        assert!(app.state.todo_panel.is_some());
        let frame = view::composer_frame(&app.state, "fixture", "folder");
        let text = frame.live.iter().map(Line::plain).collect::<Vec<_>>();
        let notice = text
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            notice.contains(crate::tui::view::todo::NO_PLAN),
            "{width}x{height}: {text:#?}"
        );
        assert!(
            text.iter()
                .any(|row| row.contains("Esc") && row.contains("close"))
        );
        assert!(!text.iter().any(|row| row.contains("scroll")));
        let line = frame
            .live
            .iter()
            .find(|row| row.plain().contains("No plan"))
            .unwrap();
        assert!(
            line.spans
                .iter()
                .any(|span| span.style == crate::tui::theme::DIM)
        );
        app.input(Decoded::Key(key(27))).unwrap();
        assert!(app.state.todo_panel.is_none());
    }
    assert!(fixture.finish().is_empty());
}
