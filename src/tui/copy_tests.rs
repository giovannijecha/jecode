use super::*;
use crate::{
    config::{Settings, Store},
    json::Value,
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, Response},
    tools::Tools,
};
use std::sync::mpsc;
use std::thread;

fn app(directory: &Directory, fixture: &HttpFixture) -> App {
    App::new(
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        ),
        SessionConfig {
            store: Store::new(directory.path().join("config")),
            settings: Settings::new("fixture-key".into(), "fixture/model".into()).unwrap(),
            bash: crate::tools::find_bash().unwrap(),
        },
        None,
    )
}
fn source(app: &mut App, text: &str) {
    app.archive.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string(text)),
    ]));
    app.state.message(Kind::Assistant, text);
}
fn key(app: &mut App, code: u16, modifiers: u8) {
    assert!(
        !app.input(Decoded::Key(terminal::Key {
            code,
            modifiers,
            character: 0
        }))
        .unwrap()
    );
}

#[test]
fn copy_selector_preserves_the_payload_draft_and_conversation() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    source(
        &mut app,
        "Intro\r\n```rust\r\n  let café = 1;  \r\n```\r\n> quoted\r\n",
    );
    let before = app.archive.messages.lock().unwrap().clone();
    app.state.editor.replace("unsent draft".into());
    app.state.editor.cursor = 4;
    app.dispatch("/copy".into()).unwrap();
    let selector = app.state.selector.as_ref().unwrap();
    assert_eq!(selector.options.len(), 3);
    assert_eq!(selector.options[0].name, "Whole response");
    key(&mut app, 40, 0);
    key(&mut app, 13, 0);
    app.poll().unwrap();
    assert_eq!(app.copied_text.as_deref(), Some("  let café = 1;  \r\n"));
    assert_eq!(app.state.editor.text, "unsent draft");
    assert_eq!(app.state.editor.cursor, 4);
    assert!(
        app.state
            .copy_notice
            .as_ref()
            .unwrap()
            .text
            .contains("Copied")
    );
    assert_eq!(*app.archive.messages.lock().unwrap(), before);
    assert!(app.archive.events.lock().unwrap().is_empty());
    assert!(fixture.finish().is_empty());
}

#[test]
fn copy_during_work_is_immediate_and_keeps_its_snapshot_until_selection() {
    let directory = Directory::new();
    let (release, waiting) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![Response::GatedStream {
        head: "data: {\"choices\":[{\"delta\":{\"content\":\"New response\"}}]}\n\n".into(),
        tail: "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"
            .into(),
        release: waiting,
    }]);
    let mut app = app(&directory, &fixture);
    source(&mut app, "Previous completed response");
    app.dispatch("Continue working".into()).unwrap();
    app.state.editor.replace("/copy".into());
    key(&mut app, 13, 0);
    assert!(app.copy_selector_open());
    assert!(
        super::view::frame(&app.state, "fixture/model", "fixture")
            .live
            .iter()
            .any(|line| line.plain().contains("Esc closes menu"))
    );
    assert!(app.state.queue.messages.is_empty());
    app.state.queue.push("/help".into()).unwrap();
    app.state.editor.replace("draft kept".into());
    release.send(()).unwrap();
    let started = Instant::now();
    while app.worker.is_some() {
        app.poll().unwrap();
        assert!(started.elapsed() < Duration::from_secs(8));
        thread::sleep(Duration::from_millis(5));
    }
    assert!(app.copy_selector_open());
    assert_eq!(app.copy_targets[0].text, "Previous completed response");
    assert_eq!(app.state.queue.messages.len(), 1);
    key(&mut app, 13, 0);
    app.poll().unwrap();
    assert_eq!(
        app.copied_text.as_deref(),
        Some("Previous completed response")
    );
    assert_eq!(app.state.editor.text, "draft kept");
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn closing_copy_while_working_does_not_cancel_the_agent_or_clear_the_draft() {
    let directory = Directory::new();
    let (release, waiting) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![Response::GatedStream {
        head: "".into(),
        tail: "data: {\"choices\":[{\"delta\":{\"content\":\"Done\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into(),
        release: waiting,
    }]);
    let mut app = app(&directory, &fixture);
    source(&mut app, "Completed");
    app.dispatch("Work".into()).unwrap();
    app.state.editor.replace("kept".into());
    for (code, modifiers) in [(27, 0), (67, 4)] {
        app.dispatch("/copy".into()).unwrap();
        key(&mut app, code, modifiers);
        assert!(app.state.selector.is_none());
        assert!(app.worker.is_some());
        assert!(!app.state.activity.as_ref().unwrap().stopping);
        assert_eq!(app.state.editor.text, "kept");
    }
    release.send(()).unwrap();
    let started = Instant::now();
    while app.worker.is_some() {
        app.poll().unwrap();
        assert!(started.elapsed() < Duration::from_secs(8));
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn empty_response_invalid_arguments_and_recovered_partial_are_not_copied() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.event(crate::events::Event::Streaming {
        text: "Partial only".into(),
    });
    app.state.finish_stream();
    app.dispatch("/copy".into()).unwrap();
    assert!(app.state.selector.is_none());
    assert!(
        app.state
            .copy_notice
            .as_ref()
            .unwrap()
            .text
            .contains("No completed")
    );
    app.dispatch("/copy extra".into()).unwrap();
    assert_eq!(app.state.copy_notice.as_ref().unwrap().text, "Usage: /copy");
    assert!(app.copied_text.is_none());
    assert!(fixture.finish().is_empty());
}

#[test]
fn searchable_copy_menu_and_small_dimensions_preserve_full_text() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let long = "界🙂  ".repeat(2000);
    source(
        &mut app,
        &format!(
            "{}\n```\n{long}\n```",
            (0..10)
                .map(|i| format!("```language{i}\ncode{i}\n```\n"))
                .collect::<String>()
        ),
    );
    app.dispatch("/copy".into()).unwrap();
    assert!(app.state.selector.as_ref().unwrap().searchable);
    app.input(Decoded::Text("language7".into())).unwrap();
    assert_eq!(app.state.selector.as_ref().unwrap().filtered.len(), 1);
    for (width, height) in [(80, 24), (15, 6), (2, 2)] {
        app.state.width = width;
        app.state.height = height;
        let frame = super::view::lower(
            &app.state,
            "fixture/model",
            "fixture",
            vec![],
            false,
            height,
            false,
        );
        assert!(frame.live.len() <= height);
    }
    key(&mut app, 13, 0);
    app.poll().unwrap();
    assert_eq!(app.copied_text.as_deref(), Some("code7\n"));
    app.dispatch("/copy".into()).unwrap();
    assert_eq!(app.copy_targets.last().unwrap().text, format!("{long}\n"));
    assert!(fixture.finish().is_empty());
}

#[test]
fn resumed_copy_uses_completed_session_source_and_new_does_not_reuse_it() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut first = app(&directory, &fixture);
    first
        .agent
        .as_mut()
        .unwrap()
        .enable_sessions(home.path())
        .unwrap();
    first.persistence = first.agent.as_ref().unwrap().sessions();
    source(&mut first, "Saved complete\r\n```\r\n  exact  \r\n```\r\n");
    first
        .agent
        .as_mut()
        .unwrap()
        .prepare_turn("Interrupted later request")
        .unwrap();
    first
        .persistence
        .as_ref()
        .unwrap()
        .partial("Incomplete later response")
        .unwrap();
    let id = first.persistence.as_ref().unwrap().id();
    drop(first);

    let mut current = app(&directory, &fixture);
    current
        .agent
        .as_mut()
        .unwrap()
        .enable_sessions(home.path())
        .unwrap();
    current.persistence = current.agent.as_ref().unwrap().sessions();
    current.resume_session(&id);
    let before = current.persistence.as_ref().unwrap().snapshot();
    assert!(before.events.iter().any(|event| {
        event.get("partial_text").and_then(Value::as_str) == Some("Incomplete later response")
    }));
    current.dispatch("/copy".into()).unwrap();
    assert!(
        current.copy_targets[0]
            .text
            .starts_with("Saved complete\r\n")
    );
    key(&mut current, 40, 0);
    key(&mut current, 13, 0);
    current.poll().unwrap();
    assert_eq!(current.copied_text.as_deref(), Some("  exact  \r\n"));
    current.agent.as_ref().unwrap().save_session().unwrap();
    let after = current.persistence.as_ref().unwrap().snapshot();
    assert_eq!(after.messages, before.messages);
    assert_eq!(after.events, before.events);

    current.copy_job = Some((
        "Discarded".into(),
        crate::clipboard::Job::fixture(Ok(crate::clipboard::Delivery::Confirmed)),
    ));
    current.dispatch("/new".into()).unwrap();
    assert!(current.copy_job.is_none());
    assert!(current.state.copy_notice.is_none());
    current.dispatch("/copy".into()).unwrap();
    assert!(!current.copy_selector_open());
    assert!(
        current
            .state
            .copy_notice
            .as_ref()
            .unwrap()
            .text
            .contains("No completed")
    );
    assert!(fixture.finish().is_empty());
}

#[test]
fn clipboard_failure_is_visible_masked_and_keeps_the_session_warning() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.archive.redactor = crate::redact::Redactor::new("fixture-key".into());
    let warning = "Recovered interrupted work";
    app.state.notice = Some(Feedback::result(Kind::Warning, warning));
    app.copy_job = Some((
        "Whole response".into(),
        crate::clipboard::Job::fixture(Err("fixture-key: simulated failure".into())),
    ));
    assert!(app.poll_copy());
    let notice = app.state.copy_notice.as_ref().unwrap();
    assert!(matches!(notice.kind, Kind::Error));
    assert!(notice.text.contains("Copy failed"));
    assert!(!notice.text.contains("fixture-key"));
    assert_eq!(app.state.notice.as_ref().unwrap().text, warning);
    assert!(!app.state.expire_feedback(Instant::now()));
    app.state.feedback_input(false);
    assert!(app.state.copy_notice.is_none());
    assert_eq!(app.state.notice.as_ref().unwrap().text, warning);
    assert!(fixture.finish().is_empty());
}
