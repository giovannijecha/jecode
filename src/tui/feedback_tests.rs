use super::*;
use crate::{
    events::Event,
    json::Value,
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, Response, completion},
    tools::Tools,
};

fn app(home: &Directory, project: &Directory, endpoint: &str) -> App {
    let mut agent = Agent::new(
        OpenRouter::fixture(endpoint.into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    App::new(agent, tests::config(project), None)
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

fn live(app: &App) -> String {
    view::frame(&app.state, "fixture/model", "fixture")
        .live
        .iter()
        .map(line::Line::plain)
        .collect::<Vec<_>>()
        .join("\n")
}

fn wait_job(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.job.is_some() {
        app.poll().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn automatic_queue_dispatch_and_catalog_loading_keep_an_unread_error() {
    for prompt in ["follow up", "/model", "/new"] {
        let home = Directory::new();
        let project = Directory::new();
        let (release, gate) = std::sync::mpsc::channel();
        let mut responses = vec![Response::GatedStream {
            head: "data: {\"choices\":[{\"delta\":{\"content\":\"First \"},\"finish_reason\":null}]}\n\n".into(),
            tail: "data: {\"choices\":[{\"delta\":{\"content\":\"answer\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into(),
            release: gate,
        }];
        if prompt != "/new" {
            responses.push(Response::Json(
                200,
                if prompt == "/model" {
                    interaction_tests::catalog()
                } else {
                    completion("Second answer", vec![])
                },
            ));
        }
        let fixture = HttpFixture::streaming(responses);
        let mut app = app(&home, &project, &fixture.endpoint);
        app.dispatch("first request".into()).unwrap();
        app.state.queue.push(prompt).unwrap();
        app.state.notify(Feedback::result(
            Kind::Error,
            "Autosave failed after queuing",
        ));
        app.state
            .notify_copy(Feedback::result(Kind::Error, "Copy error after queuing"));
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.worker.is_some() || app.job.is_some() || !app.state.queue.messages.is_empty() {
            app.poll().unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            app.state.notice.as_ref().unwrap().text,
            "Autosave failed after queuing",
            "{prompt}"
        );
        assert_eq!(
            app.state.copy_notice.as_ref().unwrap().text,
            "Copy error after queuing",
            "{prompt}"
        );
        if prompt == "/model" {
            assert!(matches!(
                app.state.selector.as_ref().unwrap().purpose,
                selector::Purpose::Models { .. }
            ));
        }
        assert_eq!(fixture.finish().len(), if prompt == "/new" { 1 } else { 2 });
    }
}

#[test]
fn autosave_recovery_clears_bookkeeping_but_keeps_the_error_until_input() {
    let home = Directory::new();
    let project = Directory::new();
    let mut app = app(&home, &project, "http://127.0.0.1:1/chat/completions");
    app.archive.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Saved request")),
    ]));
    app.agent.as_ref().unwrap().save_session().unwrap();
    let bucket = std::fs::read_dir(home.path().join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let journal = bucket.join(format!("{}.jsonl", app.persistence.as_ref().unwrap().id()));
    let original = std::fs::metadata(&journal).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&journal, readonly).unwrap();
    app.state.editor.replace("new draft".into());
    let failed = app.persist_input(true);
    std::fs::set_permissions(&journal, original).unwrap();
    assert!(failed);
    let message = app.state.notice.as_ref().unwrap().text.clone();
    assert!(message.contains("Autosave failed"));
    assert!(!app.persist_input(true));
    assert!(app.save_error.is_none());
    assert_eq!(app.state.notice.as_ref().unwrap().text, message);
    key(&mut app, 37, 0);
    assert!(app.state.notice.is_none());
}

#[test]
fn worker_completion_and_connection_events_keep_a_pending_session_error_visible() {
    let home = Directory::new();
    let project = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Completed answer", vec![]))]);
    let mut app = app(&home, &project, &fixture.endpoint);
    app.dispatch("Request".into()).unwrap();
    app.state
        .notify(Feedback::result(Kind::Error, "Pending session error"));
    app.state.event(Event::Recovering {
        attempt: 1,
        delay: Duration::from_secs(1),
        error: "Connection error".into(),
    });
    app.state.event(Event::RecoveryFinished);
    app.state.event(Event::Maintenance {
        text: "Compacting context".into(),
    });
    app.state.event(Event::ContextCompacted {
        text: "Context compacted".into(),
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.worker.is_some() {
        app.poll().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        app.state.notice.as_ref().unwrap().text,
        "Pending session error"
    );
    assert!(live(&app).contains("Pending session error"));
    assert!(
        app.state
            .rows()
            .iter()
            .flatten()
            .any(|line| line.plain() == "Completed answer")
    );
    fixture.finish();
}

#[test]
fn closing_selectors_and_settings_is_silent_and_retains_the_draft() {
    let home = Directory::new();
    let project = Directory::new();
    let mut app = app(&home, &project, "http://127.0.0.1:1/chat/completions");
    app.state.editor.replace("kept\ndraft".into());
    app.state.editor.cursor = 2;
    for _ in 0..3 {
        app.local_start("/model");
        app.state.selector = Some(selector::Selector::loading());
        key(&mut app, 27, 0);
        assert!(app.state.selector.is_none());
        assert!(app.state.notice.is_none());
        assert!(!live(&app).contains("Selection cancelled"));
    }
    app.dispatch("/settings".into()).unwrap();
    app.input(Decoded::Text("3".into())).unwrap();
    key(&mut app, 27, 0);
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Settings
    ));
    assert!(app.state.notice.is_none());
    app.input(Decoded::Text("4".into())).unwrap();
    assert!(app.state.selector.is_none());
    assert!(app.state.notice.is_none());
    assert_eq!(app.state.editor.text, "kept\ndraft");
    assert_eq!(app.state.editor.cursor, 2);
    assert!(app.state.rows().iter().all(Vec::is_empty));
    assert!(
        app.agent
            .as_ref()
            .unwrap()
            .sessions()
            .unwrap()
            .store()
            .list()
            .unwrap()
            .sessions
            .is_empty()
    );
}

#[test]
fn help_and_temporary_info_close_without_sending_or_losing_a_preserved_draft() {
    let home = Directory::new();
    let project = Directory::new();
    let mut app = app(&home, &project, "http://127.0.0.1:1/chat/completions");
    app.state.editor.replace("first\nsecond".into());
    app.state.editor.cursor = 2;
    for command in ["/help", "/TMP"] {
        app.dispatch(command.into()).unwrap();
        assert!(app.state.information.is_some());
        assert!(
            view::frame(&app.state, "fixture", "fixture")
                .cursor
                .is_none()
        );
        assert!(app.state.rows().iter().all(Vec::is_empty));
        assert!(
            !app.state
                .expire_feedback(Instant::now() + Duration::from_secs(60))
        );
        assert!(app.state.information.is_some());
        key(&mut app, 13, 0);
        assert!(app.state.information.is_none());
        assert!(app.worker.is_none());
        assert_eq!(app.state.editor.text, "first\nsecond");
        assert_eq!(app.state.editor.cursor, 2);
        app.dispatch(command.into()).unwrap();
        key(&mut app, 67, 4);
        assert!(app.state.information.is_none());
        assert_eq!(app.state.editor.text, "first\nsecond");
    }
    app.help();
    app.input(Decoded::Text("α🙂\n".into())).unwrap();
    assert!(app.state.information.is_none());
    assert_eq!(app.state.editor.text, "fiα🙂\nrst\nsecond");
    assert_eq!(app.state.editor.cursor, "fiα🙂\n".len());
    assert!(app.worker.is_none());
}

#[test]
fn expired_feedback_is_redrawn_while_idle_and_typing_dismisses_both_result_slots() {
    let home = Directory::new();
    let project = Directory::new();
    let mut app = app(&home, &project, "http://127.0.0.1:1/chat/completions");
    let before = Instant::now() - Duration::from_secs(5);
    app.state.notice = Some(Feedback::at(Kind::Notice, "Previous session", before));
    assert!(app.poll().unwrap());
    assert!(app.state.notice.is_none());
    app.state.notice = Some(Feedback::result(Kind::Notice, "Defaults saved"));
    app.state.copy_notice = Some(Feedback::result(Kind::Notice, "Copied"));
    app.input(Decoded::Text("draft".into())).unwrap();
    assert!(app.state.notice.is_none());
    assert!(app.state.copy_notice.is_none());
    app.dispatch("/tmp unknown".into()).unwrap();
    assert!(app.state.notice.as_ref().unwrap().text.contains("Usage:"));
    assert!(
        !app.state
            .expire_feedback(Instant::now() + Duration::from_secs(60))
    );
    key(&mut app, 37, 0);
    assert!(app.state.notice.is_none());
    assert_eq!(app.state.editor.text, "draft");
}

#[test]
fn catalog_and_key_validation_use_one_correct_loading_title() {
    let home = Directory::new();
    let project = Directory::new();
    let fixture = HttpFixture::new(vec![(200, interaction_tests::catalog())]);
    let mut app = app(&home, &project, &fixture.endpoint);
    app.dispatch("/model".into()).unwrap();
    assert_eq!(live(&app).matches("Loading model catalog…").count(), 1);
    assert!(app.state.notice.is_none());
    wait_job(&mut app);
    assert!(!live(&app).contains("Loading model catalog…"));
    key(&mut app, 27, 0);
    assert!(app.state.notice.is_none());
    fixture.finish();

    let fixture = HttpFixture::new(vec![(200, Value::object([("data", Value::Null)]))]);
    app.agent
        .as_mut()
        .unwrap()
        .replace_client(OpenRouter::fixture(fixture.endpoint.clone()));
    app.dispatch("/settings".into()).unwrap();
    app.input(Decoded::Text("3".into())).unwrap();
    app.input(Decoded::Text("isolated-key".into())).unwrap();
    key(&mut app, 13, 0);
    assert_eq!(live(&app).matches("Checking OpenRouter key…").count(), 1);
    assert!(!live(&app).contains("Loading model catalog"));
    assert!(app.state.notice.is_none());
    wait_job(&mut app);
    fixture.finish();
}

#[test]
fn resume_keeps_conversation_and_audit_records_without_reviving_old_ui_messages() {
    let home = Directory::new();
    let project = Directory::new();
    let mut first = app(&home, &project, "http://127.0.0.1:1/chat/completions");
    for (role, text) in [("user", "Saved question"), ("assistant", "Saved answer")] {
        first.archive.messages.lock().unwrap().push(Value::object([
            ("role", Value::string(role)),
            ("content", Value::string(text)),
        ]));
    }
    first.archive.events.lock().unwrap().push(Value::object([
        ("type", Value::string("turn_error")),
        ("after_message", Value::number(3)),
        ("partial_text", Value::string("Recovered partial")),
        ("error", Value::string("Old connection error")),
        ("kind", Value::string("error")),
    ]));
    first.dispatch("/help".into()).unwrap();
    first.local_start("/model");
    first.local_finish_quiet(
        Kind::Warning,
        "Selection cancelled · previous settings kept",
        vec![],
    );
    let id = first.persistence.as_ref().unwrap().id();
    drop(first);
    let mut resumed = app(&home, &project, "http://127.0.0.1:1/chat/completions");
    resumed.resume_session(&id);
    let rows = resumed
        .state
        .rows()
        .iter()
        .flatten()
        .map(line::Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rows.contains("Saved answer"));
    assert!(rows.contains("Recovered partial"));
    for old in [
        "Old connection error",
        "Partial response",
        "Selection cancelled",
        "Commands and controls",
    ] {
        assert!(!rows.contains(old));
        assert!(!live(&resumed).contains(old));
    }
    assert!(resumed.state.information.is_none());
    let saved = resumed.persistence.as_ref().unwrap().snapshot();
    assert!(saved.events.iter().any(|event| event.get("error").and_then(Value::as_str) == Some("Old connection error")));
    assert!(
        saved
            .events
            .iter()
            .any(|event| event.get("command").and_then(Value::as_str) == Some("/help"))
    );
}
