use super::*;
use crate::providers::openai_account::{Progress, Response, client};
use crate::session::{End, Session, history::Step, tests};
use crate::tls::NetworkError;
use std::sync::{Arc, Mutex};

struct Summarizer {
    requests: Arc<Mutex<Vec<String>>>,
    fail: bool,
    catalog: Option<crate::providers::openai_account::catalog::Catalog>,
}
impl Backend for Summarizer {
    fn login(
        &mut self,
        _: &Budget<'_>,
        _: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        Ok(())
    }
    fn catalog(
        &mut self,
        _: &Budget<'_>,
    ) -> Result<Option<crate::providers::openai_account::catalog::Catalog>, client::Error> {
        Ok(self.catalog.clone())
    }
    fn generate(
        &mut self,
        request: &Request,
        _: &Budget<'_>,
        _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        self.requests
            .lock()
            .unwrap()
            .push(request.encode(MAX_CONTEXT)?);
        if request.instructions.starts_with("Summarize") {
            assert!(request.tools.is_empty());
            if self.fail {
                return Err(NetworkError::io(
                    crate::tls::IoOperation::ReadRecordHeader,
                    &std::io::Error::from(std::io::ErrorKind::ConnectionReset),
                )
                .into());
            }
            Ok(tests::response(
                "Retain the user's original goal and completed change in settings.rs.",
                Status::Completed,
            ))
        } else {
            Ok(tests::response("Continued", Status::Completed))
        }
    }
}
fn history() -> History {
    let mut history = History::default();
    history.projection.limit_bytes = 65536;
    for n in 0..4 {
        history.begin(format!("task-{n}")).unwrap();
        let answer = "a".repeat(28_000);
        let turn = history.turns.last_mut().unwrap();
        turn.steps.push(Step {
            text: answer.clone(),
            response: Some(tests::response(&answer, Status::Completed)),
            accepted: true,
            ..Default::default()
        });
        turn.end = Some(End::Complete);
        turn.outcome = "Complete".into();
    }
    history
}
#[test]
fn uncompactable_empty_turn_does_not_loop() {
    let mut history = History::default();
    history.begin("original".into()).unwrap();
    history.turns[0].prompt = "x".repeat(MAX_CONTEXT);
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    history.begin("next action".into()).unwrap();
    let (events, _received) = std::sync::mpsc::sync_channel(8);
    let context = Context {
        events,
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        stopped: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        guidance: Arc::new(crate::session::queue::Pending::default()),
        next_effect: std::sync::atomic::AtomicU64::new(1),
        effect_gate: None,
    };
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut backend = Summarizer {
        requests: observed.clone(),
        fail: false,
        catalog: None,
    };
    assert_eq!(
        ensure(
            &mut backend,
            &mut history,
            &context,
            Model::Luna,
            false,
            &mut Metrics::default()
        ),
        Err(Failure::HistoryLimit)
    );
    assert!(observed.lock().unwrap().is_empty());
}
fn finish(session: &mut Session) -> End {
    loop {
        match tests::next(session) {
            Event::Finished(end, _) => return end,
            Event::ContextReport(_) | Event::Text(_) | Event::RequestStarted => {}
            _ => panic!("unexpected compaction event"),
        }
    }
}
#[test]
fn compaction_projects_a_summary_and_keeps_recent_turns_without_tools() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let selected = Model::new("account-model", Some("low")).unwrap();
    let mut session = Session::with_history(
        selected,
        Summarizer {
            requests: observed.clone(),
            fail: false,
            catalog: None,
        },
        None,
        history(),
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.compact());
    assert_eq!(finish(&mut session), End::Complete);
    assert!(session.submit("continue"));
    assert_eq!(finish(&mut session), End::Complete);
    let requests = observed.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        let value = crate::json::parse(request, Default::default()).unwrap();
        assert_eq!(
            value.get("model").and_then(crate::json::Value::text),
            Some("account-model")
        );
        assert_eq!(
            value
                .get("reasoning")
                .and_then(|v| v.get("effort"))
                .and_then(crate::json::Value::text),
            Some("low")
        );
    }
    assert!(requests[0].contains("task-0") && requests[0].contains("task-1"));
    assert!(requests[0].contains("task-2"));
    assert!(!requests[1].contains("task-0"));
    assert!(requests[1].contains("settings.rs") && requests[1].contains("continue"));
}
#[test]
fn automatic_compaction_and_followup_share_the_selected_pair() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let selected = Model::new("account-model", Some("high")).unwrap();
    let mut session = Session::with_history(
        selected,
        Summarizer {
            requests: observed.clone(),
            fail: false,
            catalog: None,
        },
        None,
        history(),
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.submit("continue"));
    assert_eq!(finish(&mut session), End::Complete);
    let requests = observed.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        let value = crate::json::parse(request, Default::default()).unwrap();
        assert_eq!(
            value.get("model").and_then(crate::json::Value::text),
            Some("account-model")
        );
        assert_eq!(
            value
                .get("reasoning")
                .and_then(|v| v.get("effort"))
                .and_then(crate::json::Value::text),
            Some("high")
        );
    }
}
#[test]
fn failed_compaction_is_not_automatically_retried_and_original_projection_remains() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history(
        Model::Luna,
        Summarizer {
            requests: observed.clone(),
            fail: true,
            catalog: None,
        },
        None,
        history(),
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.compact());
    assert!(matches!(
        finish(&mut session),
        End::Failed(Failure::Account(_))
    ));
    assert!(session.submit("new task"));
    assert_eq!(finish(&mut session), End::Complete);
    let requests = observed.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.contains("Summarize this bounded portion"))
            .count(),
        1
    );
    drop(requests);
    assert!(session.inspect_context());
    let Event::ContextReport(report) = tests::next(&mut session) else {
        panic!("missing context report")
    };
    assert!(report.contains("5 canonical turns / 0 turns and 0 steps summarized"));
    assert!(report.contains("unknown input tokens"));
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn fresh_unsupported_pair_rejects_manual_compaction_without_a_checkpoint() {
    let catalog = crate::providers::openai_account::catalog::Catalog::parse(br#"{"models":[{"slug":"replacement","visibility":"list","supported_reasoning_levels":[{"effort":"high"}]}]}"#).unwrap();
    for selected in [Model::Luna, Model::new("replacement", Some("low")).unwrap()] {
        let fixture = crate::state::tests::Fixture::new();
        let Some(store) = fixture.store() else { return };
        let mut saved = crate::session::persistence::create(&store, selected, None).unwrap();
        let mut prepared = history();
        saved.turns = std::mem::take(&mut prepared.turns);
        saved.projection = prepared.projection;
        saved.checkpoint().unwrap();
        let id = saved.record.as_ref().unwrap().id().to_owned();
        let sessions = store.directory("sessions").unwrap();
        let filename = format!("{id}.json");
        let before = sessions.read(&filename, 2 * 1024 * 1024).unwrap().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut session = Session::with_history(
            selected,
            Summarizer {
                requests: requests.clone(),
                fail: false,
                catalog: Some(catalog.clone()),
            },
            None,
            saved,
        )
        .unwrap();
        assert!(matches!(tests::next(&mut session), Event::Restored { .. }));
        assert!(matches!(tests::next(&mut session), Event::CatalogLoaded(_)));
        assert!(matches!(tests::next(&mut session), Event::Ready));
        assert!(session.selection_unavailable());
        assert!(!session.submit("keep this draft"));
        assert!(!session.compact());
        assert!(session.ready());
        assert!(session.poll().is_none());
        assert!(requests.lock().unwrap().is_empty());
        assert_eq!(
            sessions.read(&filename, 2 * 1024 * 1024).unwrap().unwrap(),
            before
        );
        assert!(session.inspect_context());
        let Event::ContextReport(report) = tests::next(&mut session) else {
            panic!("missing context report")
        };
        assert!(
            report.contains("4 canonical turns / 0 turns and 0 steps summarized"),
            "{report}"
        );

        let replacement = Model::new("replacement", Some("high")).unwrap();
        assert!(session.set_model(replacement));
        assert!(
            matches!(tests::next(&mut session), Event::ModelChanged(model) if model == replacement)
        );
        assert!(session.compact());
        assert_eq!(finish(&mut session), End::Complete);
        assert_eq!(requests.lock().unwrap().len(), 1);
    }
}

#[test]
fn unknown_effort_capability_does_not_block_manual_compaction() {
    let catalog = crate::providers::openai_account::catalog::Catalog::parse(
        br#"{"models":[{"slug":"known-model","visibility":"list"}]}"#,
    )
    .unwrap();
    let selected = Model::new("known-model", Some("high")).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history(
        selected,
        Summarizer {
            requests: requests.clone(),
            fail: false,
            catalog: Some(catalog),
        },
        None,
        history(),
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::CatalogLoaded(_)));
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(!session.selection_unavailable());
    assert!(session.compact());
    assert_eq!(finish(&mut session), End::Complete);
    assert_eq!(requests.lock().unwrap().len(), 1);
}
#[test]
fn oversized_original_projection_is_compacted_in_bounded_slices() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut history = History::default();
    history.projection.limit_bytes = 65536;
    history.begin("retain the objective".into()).unwrap();
    for _ in 0..3 {
        let answer = "x".repeat(700_000);
        history.turns[0].steps.push(Step {
            text: answer.clone(),
            response: Some(tests::response(&answer, Status::Completed)),
            accepted: true,
            ..Default::default()
        });
    }
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    let mut session = Session::with_history(
        Model::Luna,
        Summarizer {
            requests: observed.clone(),
            fail: false,
            catalog: None,
        },
        None,
        history,
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.submit("continue"));
    assert_eq!(finish(&mut session), End::Complete);
    let requests = observed.lock().unwrap();
    assert!(requests.len() >= 3);
    assert!(requests.iter().all(|request| request.len() <= MAX_CONTEXT));
    assert!(
        requests.last().unwrap().contains("retain the objective")
            || requests.last().unwrap().contains("original goal")
    );
}
#[test]
fn boundary_guidance_stays_once_in_live_projection() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut history = History::default();
    history.begin("original objective".into()).unwrap();
    let answer = "completed step".repeat(500);
    history.turns[0].steps.push(Step {
        text: answer.clone(),
        response: Some(tests::response(&answer, Status::Completed)),
        accepted: true,
        ..Default::default()
    });
    history.turns[0]
        .guidance
        .push(super::super::queue::Guidance {
            after_step: 1,
            text: "later guidance".into(),
        });
    let mut session = Session::with_history(
        Model::Luna,
        Summarizer {
            requests: observed.clone(),
            fail: false,
            catalog: None,
        },
        None,
        history,
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.compact());
    assert_eq!(finish(&mut session), End::Complete);
    assert!(session.submit("next user turn"));
    assert_eq!(finish(&mut session), End::Complete);
    let requests = observed.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].matches("later guidance").count(), 1); // instruction only
    assert_eq!(requests[1].matches("later guidance").count(), 1);
    assert!(requests[1].contains("original goal"));
}
#[test]
fn ended_uncertain_turn_does_not_pin_later_compaction() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut history = History::default();
    history.begin("first objective".into()).unwrap();
    let response =
        crate::session::tool_tests::calls_response(vec![crate::session::tool_tests::call(
            "call-1",
            "read_file",
            r#"{"path":"notes.txt"}"#,
        )]);
    history.turns[0].steps.push(Step {
        response: Some(response),
        accepted: true,
        results: vec![crate::session::history::Receipt {
            call_id: "call-1".into(),
            output: "not executed".into(),
            summary: "Not executed".into(),
            image: None,
        }],
        ..Default::default()
    });
    history.turns[0].end = Some(End::Failed(Failure::Cancelled));
    history.turns[0].outcome = "Cancelled; call not executed".into();
    history.begin("later objective".into()).unwrap();
    let answer = "x".repeat(100_000);
    history.turns[1].steps.push(Step {
        text: answer.clone(),
        response: Some(tests::response(&answer, Status::Completed)),
        accepted: true,
        ..Default::default()
    });
    history.turns[1].end = Some(End::Complete);
    history.turns[1].outcome = "Complete".into();
    let mut session = Session::with_history(
        Model::Luna,
        Summarizer {
            requests: observed.clone(),
            fail: false,
            catalog: None,
        },
        None,
        history,
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.compact());
    assert_eq!(finish(&mut session), End::Complete);
    let requests = observed.lock().unwrap();
    assert!(requests[0].contains("call not executed"));
    assert!(requests[0].contains("Not executed"));
}
#[test]
#[cfg(any(windows, target_os = "linux"))]
fn failed_candidate_checkpoint_keeps_prior_projection_on_disk() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = crate::session::persistence::create(&store, Model::Luna, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("retain the original task".into()).unwrap();
    let answer = "x".repeat(10_000);
    history.turns[0].steps.push(Step {
        text: answer.clone(),
        response: Some(tests::response(&answer, Status::Completed)),
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    history.checkpoint().unwrap();
    let sessions = store.directory("sessions").unwrap();
    let name = format!("{id}.json");
    let before = sessions.read(&name, 16 * 1024 * 1024).unwrap().unwrap();
    history
        .fail_next_checkpoint
        .store(true, std::sync::atomic::Ordering::Release);
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history(
        Model::Luna,
        Summarizer {
            requests: observed.clone(),
            fail: false,
            catalog: None,
        },
        None,
        history,
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Restored { .. }));
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.compact());
    assert_eq!(finish(&mut session), End::Failed(Failure::Storage));
    assert_eq!(observed.lock().unwrap().len(), 1);
    assert_eq!(
        sessions.read(&name, 16 * 1024 * 1024).unwrap().unwrap(),
        before
    );
    assert!(!session.submit("must not run after failed checkpoint"));
}
#[test]
fn cancelled_and_invalid_compaction_preserve_prior_projection() {
    struct BadSummary {
        cancel: bool,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl Backend for BadSummary {
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
            budget: &Budget<'_>,
            _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
        ) -> Result<Response, client::Error> {
            assert!(request.instructions.starts_with("Summarize"));
            self.calls.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            if self.cancel {
                budget
                    .cancelled
                    .store(true, std::sync::atomic::Ordering::Release);
                return Err(NetworkError::Cancelled.into());
            }
            Ok(tests::response("", Status::Completed))
        }
    }
    for (cancel, expected) in [
        (true, Failure::Cancelled),
        (false, Failure::CompactionOutput),
    ] {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut session = Session::with_history(
            Model::Luna,
            BadSummary {
                cancel,
                calls: calls.clone(),
            },
            None,
            history(),
        )
        .unwrap();
        assert!(matches!(tests::next(&mut session), Event::Ready));
        assert!(session.compact());
        assert_eq!(finish(&mut session), End::Failed(expected));
        assert_eq!(calls.load(std::sync::atomic::Ordering::Acquire), 1);
        assert!(session.inspect_context());
        let Event::ContextReport(report) = tests::next(&mut session) else {
            panic!("missing context report")
        };
        assert!(report.contains("0 turns and 0 steps summarized"));
    }
}
#[test]
fn refused_step_text_is_preserved_when_raw_items_are_not_projectable() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut history = History::default();
    history.begin("original objective".into()).unwrap();
    let mut response =
        crate::session::tool_tests::calls_response(vec![crate::session::tool_tests::call(
            "orphan",
            "read_file",
            r#"{"path":"notes.txt"}"#,
        )]);
    response.status = Status::Refused;
    response.text = format!("I cannot complete that operation. {}", "x".repeat(1000));
    history.turns[0].steps.push(Step {
        text: response.text.clone(),
        response: Some(response),
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Refused);
    history.turns[0].outcome = "Response refused".into();
    history.begin("later completed objective".into()).unwrap();
    let answer = "y".repeat(10_000);
    history.turns[1].steps.push(Step {
        text: answer.clone(),
        response: Some(tests::response(&answer, Status::Completed)),
        accepted: true,
        ..Default::default()
    });
    history.turns[1].end = Some(End::Complete);
    history.turns[1].outcome = "Complete".into();
    let mut session = Session::with_history(
        Model::Luna,
        Summarizer {
            requests: observed.clone(),
            fail: false,
            catalog: None,
        },
        None,
        history,
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.compact());
    assert_eq!(finish(&mut session), End::Complete);
    let requests = observed.lock().unwrap();
    assert!(requests[0].contains("I cannot complete that operation."));
    assert!(requests[0].contains("Recorded refused step"));
}
