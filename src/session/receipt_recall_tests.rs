use super::*;
use crate::{
    providers::openai_account::{Progress, Request, Response, client},
    session::{End, Event, Model, Session, history::Receipt, tool_tests, worker::Backend},
    tls::Budget as NetworkBudget,
    workspace::Workspace,
};
use std::{
    ops::ControlFlow,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{Duration, Instant},
};

fn budget(cancelled: &AtomicBool) -> Budget<'_> {
    Budget {
        cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    }
}
fn admitted_page(output: &str) -> String {
    json::encode(
        &json::object([
            ("ok", Value::Bool(true)),
            (
                "source",
                string("original recorded session receipt; no source reread"),
            ),
            ("turn", number(0)),
            ("step", number(0)),
            ("receipt", number(0)),
            ("offset", number(0)),
            ("call_id", string("original-read")),
            ("call_name", string("read_file")),
            ("output", string(output)),
            ("next", Value::Null),
        ]),
        super::super::history::MAX_TEXT,
    )
    .unwrap()
}
fn captured_history(output: String) -> (crate::state::tests::Fixture, String) {
    let fixture = crate::state::tests::Fixture::new();
    let store = fixture.store().unwrap();
    let mut history =
        crate::session::persistence::create_in(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("Use the recorded evidence".into()).unwrap();
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(vec![tool_tests::call(
            "old-read",
            "read_file",
            r#"{"path":"evidence.txt"}"#,
        )])),
        results: vec![Receipt {
            call_id: "old-read".into(),
            output,
            summary: "read_file / original observation".into(),
            image: None,
        }],
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Complete);
    history.checkpoint().unwrap();
    history.projection.step = 1;
    history.projection.summary = "Old read completed; retrieve exact data at turn 0 step 0".into();
    history.checkpoint().unwrap();
    history.release_projected();
    assert_eq!(history.base_step, 1);
    drop(history);
    (fixture, id)
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn released_receipt_pages_are_exact_and_validate_coordinates() {
    let original = format!("original-{}-é-end", "x".repeat(8_000));
    let (fixture, id) = captured_history(original.clone());
    let saved = crate::session::persistence::load(&fixture.store().unwrap(), &id, true).unwrap();
    let cancelled = AtomicBool::new(false);
    let mut offset = 0;
    let mut assembled = String::new();
    loop {
        let page = execute(&saved.history, 0, 0, 0, offset, &budget(&cancelled));
        assert!(!page.failed, "{}", page.text);
        let value = json::parse(&page.text, Default::default()).unwrap();
        assert_eq!(value.get("call_id").and_then(Value::text), Some("old-read"));
        assert_eq!(
            value.get("call_name").and_then(Value::text),
            Some("read_file")
        );
        assembled.push_str(value.get("output").and_then(Value::text).unwrap());
        let Some(next_offset) = value
            .get("next")
            .and_then(|v| v.get("offset"))
            .and_then(Value::unsigned)
        else {
            break;
        };
        offset = next_offset as usize;
    }
    assert_eq!(assembled, original);
    for (turn, step, receipt, offset) in [
        (1, 0, 0, 0),
        (0, 1, 0, 0),
        (0, 0, 1, 0),
        (0, 0, 0, original.len() + 1),
        (0, 0, 0, original.find('é').unwrap() + 1),
    ] {
        assert!(
            execute(
                &saved.history,
                turn,
                step,
                receipt,
                offset,
                &budget(&cancelled)
            )
            .failed
        );
    }
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn encoded_limit_shrinks_escaped_pages_without_dropping_bytes() {
    let original = "\u{0001}".repeat(10_000);
    let (fixture, id) = captured_history(original.clone());
    let saved = crate::session::persistence::load(&fixture.store().unwrap(), &id, true).unwrap();
    let cancelled = AtomicBool::new(false);
    let mut offset = 0;
    let mut assembled = String::new();
    loop {
        let page = execute(&saved.history, 0, 0, 0, offset, &budget(&cancelled));
        assert!(!page.failed, "{}", page.text);
        assert!(page.text.len() <= crate::tools::MAX_OUTPUT);
        let value = json::parse(&page.text, Default::default()).unwrap();
        assembled.push_str(value.get("output").and_then(Value::text).unwrap());
        let Some(next_offset) = value
            .get("next")
            .and_then(|v| v.get("offset"))
            .and_then(Value::unsigned)
        else {
            break;
        };
        assert!(next_offset as usize > offset);
        offset = next_offset as usize;
    }
    assert_eq!(assembled, original);
}

struct RecallBackend {
    requests: Arc<Mutex<Vec<String>>>,
    round: usize,
}
impl Backend for RecallBackend {
    fn login(
        &mut self,
        _: &NetworkBudget<'_>,
        _: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        Ok(())
    }
    fn generate(
        &mut self,
        request: &Request,
        _: &NetworkBudget<'_>,
        _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        self.requests
            .lock()
            .unwrap()
            .push(request.encode(super::super::history::MAX_REQUEST)?);
        self.round += 1;
        if self.round == 1 {
            return Ok(tool_tests::calls_response(vec![tool_tests::call(
                "recall-now",
                "recall_receipts",
                r#"{"turn":0,"step":0}"#,
            )]));
        }
        Ok(crate::session::tests::response(
            "Recovered the original observation",
            Status::Completed,
        ))
    }
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn resumed_generation_receives_old_receipt_after_source_is_deleted_without_replay() {
    let files = crate::workspace_fixture::Fixture::new();
    files.write("evidence.txt", "Key: MAPLE-721\nScore: 4829\n");
    let workspace = Workspace::open(&files.0).unwrap();
    let cancelled = AtomicBool::new(false);
    let original = crate::tools::Prepared::parse(
        "read_file",
        &json::parse(r#"{"path":"evidence.txt"}"#, Default::default()).unwrap(),
    )
    .unwrap()
    .execute(&workspace, &budget(&cancelled));
    assert!(!original.failed);
    let (fixture, id) = captured_history(original.text);
    std::fs::remove_file(files.0.join("evidence.txt")).unwrap();
    let saved = crate::session::persistence::load(&fixture.store().unwrap(), &id, true).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history(
        Model::Luna,
        RecallBackend {
            requests: requests.clone(),
            round: 0,
        },
        Some(workspace),
        saved.history,
    )
    .unwrap();
    assert!(matches!(
        crate::session::tests::next(&mut session),
        Event::Restored { .. }
    ));
    assert!(matches!(
        crate::session::tests::next(&mut session),
        Event::Ready
    ));
    assert!(session.submit("Continue using the original recorded result."));
    let mut tools = Vec::new();
    loop {
        match crate::session::tests::next(&mut session) {
            Event::ToolStarted { name, .. } => tools.push(name),
            Event::Finished(end, _) => {
                assert_eq!(end, End::Complete);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(tools, ["recall_receipts"]);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(!requests[0].contains("MAPLE-721"));
    assert!(requests[1].contains("MAPLE-721") && requests[1].contains("4829"));
    assert!(requests[1].contains("original recorded session receipt"));
}

#[test]
fn newly_recalled_receipt_reaches_the_next_generation_under_context_pressure() {
    let files = crate::workspace_fixture::Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history = History::default();
    history.begin("Continue the evidence task".into()).unwrap();
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(vec![tool_tests::call(
            "recall-now",
            "recall_receipts",
            r#"{"turn":0,"step":0}"#,
        )])),
        results: vec![Receipt {
            call_id: "recall-now".into(),
            output: admitted_page("RECORDED-ORIGINAL-5831"),
            summary: "recall_receipts / recorded evidence".into(),
            image: None,
        }],
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Complete);
    history.projection.limit_bytes = 1;
    assert_eq!(history.pending_recall_step(), Some((0, 0)));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history(
        Model::Luna,
        RecallBackend {
            requests: requests.clone(),
            round: 1,
        },
        Some(workspace),
        history,
    )
    .unwrap();
    assert!(matches!(
        crate::session::tests::next(&mut session),
        Event::Ready
    ));
    assert!(session.submit("Use the recovered value."));
    loop {
        if let Event::Finished(end, _) = crate::session::tests::next(&mut session) {
            assert_eq!(end, End::Complete);
            break;
        }
    }
    let requests = requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "pending recall must not be summarized first"
    );
    assert!(requests[0].contains("RECORDED-ORIGINAL-5831"));
}

#[test]
fn malformed_references_and_effect_receipts_are_rejected() {
    for args in [
        r#"{"turn":-1,"step":0}"#,
        r#"{"turn":"0","step":0}"#,
        r#"{"turn":0}"#,
        r#"{"turn":0,"step":0,"offset":-1}"#,
        r#"{"turn":0,"step":0,"session_id":"other"}"#,
    ] {
        assert!(
            crate::tools::Prepared::parse(
                "recall_receipts",
                &json::parse(args, Default::default()).unwrap()
            )
            .is_err(),
            "{args}"
        );
    }
    let mut history = History::default();
    history.begin("Run a command".into()).unwrap();
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(vec![tool_tests::call(
            "effect",
            "run_command",
            r#"{"command":"echo x"}"#,
        )])),
        results: vec![Receipt {
            call_id: "effect".into(),
            output: "private command output".into(),
            summary: "run_command / completed".into(),
            image: None,
        }],
        accepted: true,
        ..Default::default()
    });
    let cancelled = AtomicBool::new(false);
    let denied = execute(&history, 0, 0, 0, 0, &budget(&cancelled));
    assert!(denied.failed);
    assert!(!denied.text.contains("private command output"));
}

#[test]
fn saved_oversized_recall_is_explicitly_deferred_without_changing_canonical_output() {
    let files = crate::workspace_fixture::Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let fixture = crate::state::tests::Fixture::new();
    let mut history =
        crate::session::persistence::create_in(&fixture.store().unwrap(), Model::Luna, None, None)
            .unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("Consume recalled evidence".into()).unwrap();
    let calls = (0..9)
        .map(|n| {
            tool_tests::call(
                &format!("recall-{n}"),
                "recall_receipts",
                r#"{"turn":0,"step":0}"#,
            )
        })
        .collect::<Vec<_>>();
    let results = (0..9)
        .map(|n| Receipt {
            call_id: format!("recall-{n}"),
            output: admitted_page(&"x".repeat(995_000)),
            summary: "recall_receipts / recorded evidence".into(),
            image: None,
        })
        .collect();
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(calls)),
        results,
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Complete);
    history.checkpoint().unwrap();
    drop(history);
    let saved = crate::session::persistence::load(&fixture.store().unwrap(), &id, true).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history(
        Model::Luna,
        RecallBackend {
            requests: requests.clone(),
            round: 1,
        },
        Some(workspace),
        saved.history,
    )
    .unwrap();
    assert!(matches!(
        super::review_tests::next_large(&mut session),
        Event::Restored { .. }
    ));
    assert!(matches!(
        super::review_tests::next_large(&mut session),
        Event::Ready
    ));
    assert!(session.submit("Continue"));
    loop {
        if let Event::Finished(end, _) = super::review_tests::next_large(&mut session) {
            assert_eq!(end, End::Complete);
            break;
        }
    }
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("Saved recall result deferred"));
    assert!(requests[0].contains("recall-0"));
    assert!(requests[0].len() <= super::super::history::MAX_REQUEST);
    drop(requests);
    drop(session);
    let reread = crate::session::persistence::load(&fixture.store().unwrap(), &id, true).unwrap();
    let saved_step = &reread.history.turns[0].steps[0];
    assert_eq!(saved_step.results.len(), 9);
    assert!(
        saved_step
            .results
            .iter()
            .all(|result| result.output == admitted_page(&"x".repeat(995_000)))
    );
}
