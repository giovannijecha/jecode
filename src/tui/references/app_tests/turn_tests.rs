//! References are message text: they reach the model, history, the queue and saved
//! conversations unchanged, also when chosen while a turn runs.

use super::files_tests::{open, workspace};
use super::*;
use crate::{
    json::Value,
    test_support::{HttpFixture, Response, completion},
    tui::{state, tests::task_content},
};
use std::sync::mpsc;

const STOP: &str =
    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";

fn gated(text: &str, release: mpsc::Receiver<()>) -> Response {
    Response::GatedStream {
        head: format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{text}\"}},\"finish_reason\":null}}]}}\n\n"
        ),
        tail: STOP.into(),
        release,
    }
}

fn connected(home: &Directory, project: &Directory, endpoint: &str) -> App {
    let mut agent = Agent::new(
        OpenRouter::fixture(endpoint.into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let mut app = App::new(agent, tests::config(project), None);
    app.state.width = 80;
    app.state.height = 32;
    app
}

fn user_rows(app: &App) -> Vec<String> {
    app.state
        .items
        .iter()
        .filter_map(|item| match item {
            state::Item::Text {
                kind: state::Kind::User,
                text,
            } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// `Use $review on @README.md `, built through both selectors.
fn compose(app: &mut App) {
    typed(app, "Use $");
    key(app, ENTER);
    typed(app, "on ");
    open(app);
    typed(app, "READ");
    key(app, ENTER);
    assert_eq!(app.state.editor.text, "Use $review on @README.md ");
    nothing_sent(app);
}

#[test]
fn references_reach_the_model_as_text_and_return_with_history_and_resume() {
    let (home, project) = (Directory::new(), workspace());
    skill(&home, "review", "Inspect a change");
    let fixture = HttpFixture::new(vec![(200, completion("Done.", vec![]))]);
    let mut app = connected(&home, &project, &fixture.endpoint);
    compose(&mut app);
    key(&mut app, ENTER);
    tests::finish(&mut app);
    let requests = fixture.finish();
    assert_eq!(
        requests.len(),
        1,
        "one request, and no file was read for it"
    );
    let request = &requests[0];
    let body = request.body.encode();
    assert!(body.contains("Use $review on @README.md"), "{body}");
    assert!(
        body.contains("names a skill as $"),
        "the catalog explains $name"
    );
    assert!(body.contains("name a workspace file as @path"));
    assert!(
        !body.contains("# Fixture"),
        "the file content is not imported"
    );
    assert!(
        app.state.history.snapshot()[0]
            .text
            .contains("Use $review on @README.md")
    );

    let id = app.persistence.as_ref().unwrap().id();
    drop(app);
    let mut resumed = connected(&home, &project, "http://127.0.0.1:1/chat/completions");
    resumed.resume_session(&id);
    assert!(
        user_rows(&resumed)
            .iter()
            .any(|row| row.contains("Use $review on @README.md")),
        "{:?}",
        user_rows(&resumed)
    );
}

#[test]
fn a_reference_chosen_during_a_turn_joins_that_turn_as_written() {
    let (home, project) = (Directory::new(), workspace());
    skill(&home, "review", "Inspect a change");
    let (release, gate) = mpsc::channel();
    let (finish, last) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![gated("Active.", gate), gated("Done.", last)]);
    let mut app = connected(&home, &project, &fixture.endpoint);
    typed(&mut app, "active");
    key(&mut app, ENTER);
    tests::wait_until(&mut app, |app| {
        app.state
            .items
            .iter()
            .any(|item| matches!(item, state::Item::Streaming { .. }))
    });
    typed(&mut app, "then $");
    assert!(
        app.state.selector.is_some(),
        "the selector opens over a turn"
    );
    key(&mut app, ENTER);
    assert_eq!(app.state.editor.text, "then $review ");
    assert!(app.state.queue.is_empty(), "choosing never sends");
    key(&mut app, ENTER);
    release.send(()).unwrap();
    tests::wait_until(&mut app, |app| {
        user_rows(app)
            .iter()
            .any(|row| row.contains("then $review"))
    });
    finish.send(()).unwrap();
    tests::finish(&mut app);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(task_content(&requests[1]).contains("then $review"));
}

#[test]
fn a_draft_and_a_queued_message_with_references_survive_save_and_resume() {
    let (home, project) = (Directory::new(), workspace());
    skill(&home, "review", "Inspect a change");
    let mut app = app(&home, &project);
    for (role, content) in [("user", "Earlier request"), ("assistant", "Earlier answer")] {
        app.archive.messages.lock().unwrap().push(Value::object([
            ("role", Value::string(role)),
            ("content", Value::string(content)),
        ]));
    }
    app.agent.as_ref().unwrap().save_session().unwrap();
    compose(&mut app);
    let queued = r#"then $review @"docs/my file.md""#;
    app.state.queue.push(queued).unwrap();
    app.persist_input(true);
    let id = app.persistence.as_ref().unwrap().id();
    drop(app);

    let mut resumed = super::app(&home, &project);
    resumed.resume_session(&id);
    assert_eq!(resumed.state.editor.text, "Use $review on @README.md ");
    assert_eq!(resumed.state.queue.paused.as_ref().unwrap().text, queued);
}
