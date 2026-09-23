use super::*;
use crate::{
    providers::openai_account::{Progress, Request, Response, Status, client},
    session::{
        self, End, Event, Failure, Session,
        history::{Receipt, Step},
    },
    tls::Budget,
};
use std::{
    ops::ControlFlow,
    sync::{Arc, Mutex},
};

struct Backend(Arc<Mutex<Vec<String>>>);

fn repeated_messages_response() -> Response {
    let crate::json::Value::Array(output) = crate::json::parse(
        r#"[{"type":"message","id":"one","status":"completed","content":[{"type":"output_text","text":"Same"}]},{"type":"message","id":"two","status":"completed","content":[{"type":"output_text","text":"Same"}]}]"#,
        Default::default(),
    ).unwrap() else { unreachable!() };
    Response {
        id: "legacy_fixture".into(),
        status: Status::Completed,
        output,
        text: "Same\n\nSame".into(),
        tool_calls: Vec::new(),
        usage: Default::default(),
    }
}

#[test]
fn changing_model_survives_resume_without_rewriting_canonical_turns() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, None).unwrap();
    history.begin("retained prompt".into()).unwrap();
    history.turns[0].end = Some(End::Complete);
    let canonical = crate::json::encode(&codec::encode(&history), LIMIT).unwrap();
    history.set_model(Model::Terra).unwrap();
    let id = history.record.as_ref().unwrap().id.clone();
    drop(history);
    let saved = load(&store, &id, true).unwrap();
    assert_eq!(saved.model, Model::Terra);
    assert_eq!(
        crate::json::encode(&codec::encode(&saved.history), LIMIT).unwrap(),
        canonical
    );
}

#[test]
fn older_joined_text_resumes_without_rewriting_saved_message_items() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, None).unwrap();
    let id = history.record.as_ref().unwrap().id.clone();
    history.begin("legacy display".into()).unwrap();
    history.turns[0].steps.push(Step {
        text: "SameSame".into(),
        response: Some(repeated_messages_response()),
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    history.checkpoint().unwrap();
    let before = codec::encode(&history);
    drop(history);
    let saved = load(&store, &id, true).unwrap();
    let step = &saved.history.turns[0].steps[0];
    assert_eq!(step.text, "SameSame");
    assert_eq!(step.response.as_ref().unwrap().text, "Same\n\nSame");
    assert_eq!(step.response.as_ref().unwrap().output.len(), 2);
    assert_eq!(codec::encode(&saved.history), before);
    saved.history.checkpoint().unwrap();
    drop(saved);
    let saved_again = load(&store, &id, true).unwrap();
    assert_eq!(codec::encode(&saved_again.history), before);
}
impl session::worker::Backend for Backend {
    fn login(
        &mut self,
        _: &Budget<'_>,
        _: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        Ok(())
    }
    fn generate(
        &mut self,
        request: &Request,
        _: &Budget<'_>,
        _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        self.0.lock().unwrap().push(request.encode(LIMIT)?);
        Ok(session::tests::response(
            "Continued without replay",
            Status::Completed,
        ))
    }
}
#[test]
fn restart_restores_canonical_receipts_and_waits_for_new_input() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else {
        return;
    };
    let mut history = create(&store, Model::Luna, None).unwrap();
    let id = history.record.as_ref().unwrap().id.clone();
    history
        .begin("remember the completed change".into())
        .unwrap();
    let response = session::tool_tests::calls_response(vec![session::tool_tests::call(
        "edit-1",
        "edit_file",
        r#"{"path":"settings.rs","old_text":"2","new_text":"3"}"#,
    )]);
    history.turns[0].steps.push(Step {
        response: Some(response),
        accepted: true,
        results: vec![Receipt {
            call_id: "edit-1".into(),
            output: r#"{"ok":true,"status":"applied"}"#.into(),
            summary: "Edited settings.rs".into(),
        }],
        ..Default::default()
    });
    history.turns[0].steps.push(Step {
        text: "Same\n\nSame".into(),
        response: Some(repeated_messages_response()),
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    history.checkpoint().unwrap();
    assert!(
        load(&store, &id, true).is_err(),
        "a concurrent owner must not acquire this session"
    );
    drop(history);
    let saved = load(&store, &id, true).unwrap();
    assert_eq!(saved.turns, 1);
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut run =
        Session::with_history(Model::Luna, Backend(observed.clone()), None, saved.history).unwrap();
    let Event::Restored {
        turns: 1, items, ..
    } = session::tests::next(&mut run)
    else {
        panic!("expected restored transcript");
    };
    assert_eq!(
        items
            .iter()
            .filter(|item| item.text == "Same\n\nSame")
            .count(),
        1
    );
    assert!(!items.iter().any(|item| item.text == "SameSame"));
    assert!(matches!(session::tests::next(&mut run), Event::Ready));
    assert!(observed.lock().unwrap().is_empty());
    assert!(run.submit("continue now"));
    loop {
        match session::tests::next(&mut run) {
            Event::Finished(End::Complete, _) => break,
            Event::Text(_) => {}
            _ => panic!("resume unexpectedly dispatched a historical tool"),
        }
    }
    let requests = observed.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("applied"));
    assert!(requests[0].contains("opaque-owned-fixture"));
    assert!(requests[0].contains("\"id\":\"one\""));
    assert!(requests[0].contains("\"id\":\"two\""));
    drop(requests);
    drop(run);
    let mut saved = load(&store, &id, true).unwrap();
    assert_eq!(saved.turns, 2);
    assert_eq!(saved.history.turns[1].metrics.requests, 1);
    saved.history.begin("third task".into()).unwrap();
    saved.history.turns[2].end = Some(End::Complete);
    saved.history.projection.through = 1;
    saved.history.projection.summary = "The settings change was applied; do not repeat it.".into();
    saved.history.checkpoint().unwrap();
    drop(saved);
    let saved = load(&store, &id, true).unwrap();
    assert_eq!(saved.history.turns.len(), 3);
    assert_eq!(
        saved.history.turns[0].steps[0].results[0].summary,
        "Edited settings.rs"
    );
    let projection = saved
        .history
        .request(Model::Luna, false)
        .unwrap()
        .encode(LIMIT)
        .unwrap();
    assert!(projection.contains("do not repeat it"));
    assert!(!projection.contains("edit-1"));
}

#[test]
fn interrupted_effect_stays_unknown_and_invalid_snapshot_is_not_repaired() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else {
        return;
    };
    let mut history = create(&store, Model::Terra, None).unwrap();
    let id = history.record.as_ref().unwrap().id.clone();
    history.begin("interrupted task".into()).unwrap();
    let response = session::tool_tests::calls_response(vec![session::tool_tests::call(
        "run-1",
        "run_command",
        r#"{"command":"test"}"#,
    )]);
    history.turns[0].steps.push(Step {
        response: Some(response),
        accepted: true,
        results: vec![Receipt {
            call_id: "run-1".into(),
            output: r#"{"status":"unknown"}"#.into(),
            summary: "Outcome unknown after interruption".into(),
        }],
        ..Default::default()
    });
    history.checkpoint().unwrap();
    drop(history);
    let mut saved = load(&store, &id, true).unwrap();
    assert!(saved.history.turns[0].outcome.contains("Interrupted"));
    assert!(
        saved
            .history
            .request(Model::Terra, true)
            .unwrap()
            .encode(LIMIT)
            .unwrap()
            .contains("unknown")
    );
    saved.history.begin("new explicit request".into()).unwrap();
    saved.history.checkpoint().unwrap();
    drop(saved);
    let saved = load(&store, &id, true).unwrap();
    assert_eq!(saved.turns, 2);
    drop(saved);
    let session_store = store.directory("sessions").unwrap();
    session_store
        .replace(&format!("{id}.json"), "{truncated")
        .unwrap();
    assert!(load(&store, &id, true).is_err());
    assert_eq!(
        session_store
            .read(&format!("{id}.json"), 100)
            .unwrap()
            .unwrap(),
        "{truncated"
    );
}

#[test]
fn failed_checkpoint_stops_before_an_approved_effect_can_be_requested() {
    struct Broken {
        file: PathBuf,
    }
    impl session::worker::Backend for Broken {
        fn login(
            &mut self,
            _: &Budget<'_>,
            _: &mut dyn FnMut(&str) -> ControlFlow<()>,
        ) -> Result<(), client::Error> {
            Ok(())
        }
        fn generate(
            &mut self,
            _: &Request,
            _: &Budget<'_>,
            _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
        ) -> Result<Response, client::Error> {
            std::fs::remove_file(&self.file).unwrap();
            std::fs::create_dir(&self.file).unwrap();
            Ok(session::tool_tests::calls_response(vec![
                session::tool_tests::call(
                    "create-1",
                    "create_file",
                    r#"{"path":"unexpected.txt","content":"bad"}"#,
                ),
            ]))
        }
    }
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else {
        return;
    };
    let workspace = crate::workspace::Workspace::open(&fixture.0).unwrap();
    let history = create(&store, Model::Luna, Some(&workspace)).unwrap();
    let record = history.record.as_ref().unwrap();
    let file = record.store.root().join(format!("{}.json", record.id));
    let mut run =
        Session::with_history(Model::Luna, Broken { file }, Some(workspace), history).unwrap();
    assert!(matches!(
        session::tests::next(&mut run),
        Event::Restored { .. }
    ));
    assert!(matches!(session::tests::next(&mut run), Event::Ready));
    assert!(run.submit("make the file"));
    assert!(matches!(
        session::tests::next(&mut run),
        Event::Finished(End::Failed(Failure::Storage), _)
    ));
    assert!(!run.submit("must not continue without durable history"));
    run.worker.take().unwrap().join().unwrap();
    assert!(
        run.poll().is_none(),
        "storage failure must not turn into a login error"
    );
    drop(run);
    assert!(!fixture.0.join("unexpected.txt").exists());
}
