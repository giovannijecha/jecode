use super::tests::{config, wait_until};
use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    effort::Effort,
    json::{self, Value},
    test_support::{Directory, HttpFixture, Response, completion, tool_call},
    tools::Tools,
};

fn app(directory: &Directory, fixture: &HttpFixture) -> App {
    App::new(
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        ),
        config(directory),
        None,
    )
}

fn submit(app: &mut App, value: &str) {
    app.input(Decoded::Text(value.into())).unwrap();
    app.input(Decoded::Key(terminal::Key {
        code: 13,
        modifiers: 0,
        character: 0,
    }))
    .unwrap();
}

/// A turn that keeps writing until `release` is sent, then one more reply.
fn gated() -> (HttpFixture, std::sync::mpsc::Sender<()>) {
    let (release, gate) = std::sync::mpsc::channel();
    let fixture = HttpFixture::streaming(vec![
        Response::GatedStream {
            head: "data: {\"choices\":[{\"delta\":{\"content\":\"First.\"},\"finish_reason\":null}]}\n\n".into(),
            tail: "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into(),
            release: gate,
        },
        Response::Json(200, completion("Second.", vec![])),
    ]);
    (fixture, release)
}

fn local_rows(app: &App) -> Vec<String> {
    app.state
        .items
        .iter()
        .filter_map(|item| match item {
            state::Item::Local {
                command,
                result: Some(result),
                ..
            } => Some(format!("{command}: {result}")),
            _ => None,
        })
        .collect()
}

#[test]
fn menus_open_during_a_turn_while_work_commands_still_wait() {
    let directory = Directory::new();
    let (fixture, release) = gated();
    let mut app = app(&directory, &fixture);
    submit(&mut app, "first");
    wait_until(&mut app, |app| app.state.status == "Writing the response…");
    submit(&mut app, "/settings");
    assert!(app.state.selector.is_some(), "the menu opens over the turn");
    assert!(app.state.queue.is_empty());
    assert_eq!(app.state.status, "Writing the response…");
    app.close_selector();
    submit(&mut app, "/init");
    assert_eq!(app.state.queue.messages(), ["/init"], "work still waits");
    release.send(()).unwrap();
    wait_until(&mut app, |app| {
        app.worker.is_none() && app.state.queue.is_empty()
    });
    wait_until(&mut app, |app| app.worker.is_none());
    assert!(
        local_rows(&app).is_empty(),
        "a menu leaves the turn's rows alone"
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2, "/init ran after the turn");
}

#[test]
fn a_model_chosen_during_a_turn_applies_from_the_next_one() {
    let directory = Directory::new();
    let (fixture, release) = gated();
    let mut app = app(&directory, &fixture);
    submit(&mut app, "first");
    wait_until(&mut app, |app| app.state.status == "Writing the response…");
    app.apply_selection("fixture/other".into(), Effort::High, false);
    let notice = app.state.notice.as_ref().unwrap().text.clone();
    assert!(notice.contains("fixture/other (high)") && notice.ends_with("from the next turn"));
    assert_eq!(app.facts().model, "fixture/other", "later menus show it");
    release.send(()).unwrap();
    wait_until(&mut app, |app| app.worker.is_none());
    assert_eq!(app.agent.as_ref().unwrap().model(), "fixture/other");
    assert_eq!(app.agent.as_ref().unwrap().effort(), Effort::High);
    let rows = local_rows(&app);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].starts_with("/model: Model: fixture/model"));
    submit(&mut app, "second");
    wait_until(&mut app, |app| app.worker.is_none());
    let requests = fixture.finish();
    let model = |index: usize| requests[index].body.get("model").cloned();
    assert_eq!(model(0), Some(Value::string("fixture/model")));
    assert_eq!(model(1), Some(Value::string("fixture/other")));
}

#[test]
fn a_question_closes_the_menu_open_over_the_turn() {
    let directory = Directory::new();
    let arguments = json::parse(
        r#"{"questions":[{"id":"scope","title":"Scope","question":"Which rows?",
        "options":[{"id":"all","label":"All rows"},{"id":"visible","label":"Visible rows"}]}]}"#,
    )
    .unwrap();
    let fixture = HttpFixture::new(vec![(
        200,
        completion("", vec![tool_call("ask-1", "ask_user", arguments)]),
    )]);
    let home = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let mut app = App::new(agent, config(&directory), None);
    submit(&mut app, "Export the report");
    submit(&mut app, "/settings");
    assert!(app.state.selector.is_some());
    wait_until(&mut app, |app| app.state.question.is_some());
    assert!(app.state.selector.is_none(), "the question takes its place");
    assert!(app.state.ask_panel.is_some());
    drop(fixture);
}
