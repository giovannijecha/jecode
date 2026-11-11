//! The approved cross-block audit, painted by the Rust renderer.
use crate::{
    ask::Request,
    backend::Model,
    json,
    permissions::Level,
    tui::{
        ask::Panel,
        feedback::{Ending, Feedback},
        selector::Selector,
        state::{Kind, State},
        theme::{self, STRONG},
        view,
    },
};

fn state(width: usize) -> State {
    let mut state = State {
        width,
        height: 24,
        ..State::default()
    };
    state.message(Kind::User, "Add CSV export and verify the result.");
    state.message(
        Kind::Assistant,
        "I mapped the report flow and checked the export.",
    );
    state
}

fn ask(width: usize) -> State {
    let mut state = state(width);
    state.editor.insert("Keep this unsent draft");
    let request = Request::parse(
        "audit-ask",
        &json::parse(
            r#"{"questions":[{"id":"rows","title":"Rows",
            "question":"Which rows should the CSV include?",
            "context":"The reports screen has active filters.","options":[
            {"id":"visible","label":"Visible rows","description":"Follow the active filters","recommended":true},
            {"id":"all","label":"All report rows","description":"Ignore filters and export everything"}]}]}"#,
        )
        .unwrap(),
    )
    .unwrap();
    let mut panel = Panel::new(&request, None);
    panel.protect_arrival(&request);
    state.question = Some(request);
    state.ask_panel = Some(panel);
    state
}

/// Twelve questions of twenty options, on the last one.
fn many_questions(width: usize) -> State {
    let mut state = state(width);
    let options = (0..20)
        .map(|id| format!(r#"{{"id":"o{id}","label":"Option {id}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let questions = (0..12)
        .map(|id| {
            format!(
                r#"{{"id":"q{id}","title":"Decision {id:02}","question":"Which option for part {id}?","options":[{options}]}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let request = Request::parse(
        "audit-many",
        &json::parse(&format!(r#"{{"questions":[{questions}]}}"#)).unwrap(),
    )
    .unwrap();
    let mut panel = Panel::new(&request, None);
    for _ in 0..11 {
        panel.switch(&request, true);
    }
    panel.entries[11].row = usize::MAX;
    state.question = Some(request);
    state.ask_panel = Some(panel);
    state
}

/// A 120-task plan in /todo.
fn long_plan(width: usize) -> State {
    let mut state = state(width);
    state.todo_plan = Some(crate::todo::Plan {
        title: "Session storage migration".into(),
        revision: 1,
        tasks: (0..120)
            .map(|index| crate::todo::Task {
                id: format!("t{index}"),
                title: format!("Task {index}"),
                status: if index < 98 { "done" } else { "pending" }.into(),
                note: String::new(),
                agent: None,
            })
            .collect(),
    });
    state.todo_panel = Some(crate::tui::todo::Panel::default());
    state
}

fn models() -> State {
    let mut state = state(40);
    let models = ["sonnet", "opus", "haiku"].map(|name| Model {
        id: format!("fixture/{name}"),
        name: name.into(),
        prompt_price: None,
        completion_price: None,
        efforts: vec![],
    });
    state.selector = Some(Selector::models(&models, false, "fixture/sonnet"));
    state
}

#[test]
#[ignore = "prints approved audit states for review using the Rust renderer"]
fn alignment() {
    let mut done = state(80);
    done.notify(Feedback::ending(
        Ending::Done,
        "1m 04s · 4 tools · ↑ 48.2k · ↓ 3.2k · 41K cached",
    ));
    let mut permissions = state(80);
    permissions.selector = Some(Selector::permissions(Level::Workspace, Level::Workspace, 0));
    let mut light = state(80);
    light.message(
        Kind::Assistant,
        "The terminal keeps its own palette after exit.",
    );
    let scenes = [
        ("Ask over a draft", ask(80), false),
        ("Ask over a draft, narrow", ask(40), false),
        (
            "Ask, twelve questions of twenty options",
            many_questions(80),
            false,
        ),
        ("Todo, 120 tasks", long_plan(80), false),
        ("Model, primary action and exit", models(), false),
        ("Permissions, recursive workspace", permissions, false),
        ("Completion statistics", done, false),
        ("Light inks", light, true),
    ];
    for (title, state, inks) in scenes {
        println!("AUDIT SCENE: {title}");
        theme::set_light(inks);
        let frame = view::frame(&state, "fixture/sonnet", r"C:\Projects\example");
        for line in frame.live {
            println!("{}", line.paint(state.width));
        }
        println!("{}", crate::tui::line::Line::new(" ", STRONG).paint(1));
    }
}
