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
        theme::{BASE, STRONG, Surface},
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
        "1m 04s · 4 tools · ↓ 3.2k · 11K cached",
    ));
    let mut permissions = state(80);
    permissions.selector = Some(Selector::permissions(Level::Workspace, Level::Workspace, 0));
    let mut light = state(80);
    light.message(
        Kind::Assistant,
        "The terminal keeps its own palette after exit.",
    );
    let scenes = [
        ("Ask over a draft", ask(80), BASE),
        ("Ask over a draft, narrow", ask(40), BASE),
        ("Model, primary action and exit", models(), BASE),
        ("Permissions, recursive workspace", permissions, BASE),
        ("Completion statistics", done, BASE),
        ("Light terminal fallback", light, Surface::rgb(12, 12, 12)),
    ];
    for (title, state, base) in scenes {
        println!("AUDIT SCENE: {title}");
        let frame = view::frame(&state, "fixture/sonnet", r"C:\Projects\example");
        for line in frame.live {
            println!("{}", line.paint_on(state.width, base));
        }
        println!("{}", crate::tui::line::Line::new(" ", STRONG).paint(1));
    }
}
