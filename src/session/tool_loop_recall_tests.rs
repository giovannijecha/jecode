use super::*;
use crate::{
    providers::openai_account::{Input, Progress, Request, Response, Status, client},
    session::{tool_tests, worker},
    tls::Budget as NetworkBudget,
    workspace::Workspace,
};
use std::{
    ops::ControlFlow,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

struct UnusedBackend;
impl Backend for UnusedBackend {
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
        assert!(
            request.instructions.starts_with("Summarize"),
            "historical generation must not replay"
        );
        Ok(crate::session::tests::handoff_response(
            request,
            "The first read completed; the second was cancelled.",
        ))
    }
}

struct ResumeProbe {
    compactions: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<(String, String)>>>,
}
impl Backend for ResumeProbe {
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
        if request.instructions.starts_with("Summarize") {
            self.compactions.fetch_add(1, Ordering::Relaxed);
            return Ok(crate::session::tests::handoff_response(
                request,
                "The source read and cancelled batch have durable receipts.",
            ));
        }
        self.seen
            .lock()
            .unwrap()
            .extend(request.input.iter().filter_map(|item| {
                if let Input::ToolResult { call_id, output } = item {
                    Some((call_id.clone(), output.clone()))
                } else {
                    None
                }
            }));
        Ok(crate::session::tests::response(
            "Continue",
            Status::Completed,
        ))
    }
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn cancelled_mixed_batch_retains_only_admitted_recall_after_checkpoint_and_resume() {
    for admitted in [false, true] {
        let files = crate::workspace_fixture::Fixture::new();
        files.write("source.txt", "changed source");
        let fixture = crate::state::tests::Fixture::new();
        let store = fixture.store().unwrap();
        let mut history =
            super::super::persistence::create_in(&store, Model::Luna, None, None).unwrap();
        let id = history.record.as_ref().unwrap().id().to_owned();
        history
            .begin("Use the original recorded read".into())
            .unwrap();
        history.turns[0].steps.push(Step {
            response: Some(tool_tests::calls_response(vec![tool_tests::call(
                "source",
                "read_file",
                r#"{"path":"source.txt"}"#,
            )])),
            results: vec![Receipt {
                call_id: "source".into(),
                output: "ORIGINAL-RECEIPT-".repeat(5000),
                summary: "read_file / source.txt".into(),
                image: None,
            }],
            accepted: true,
            ..Default::default()
        });
        let recall_args = if admitted {
            r#"{"turn":0,"step":0,"receipt":0,"expected_call_id":"source"}"#
        } else {
            r#"{"turn":0,"step":0,"receipt":0,"expected_call_id":"another-call"}"#
        };
        let response = tool_tests::calls_response(vec![
            tool_tests::call("recall", "recall_receipts", recall_args),
            tool_tests::call("sibling", "read_file", r#"{"path":"source.txt"}"#),
        ]);
        history.turns[0].steps.push(Step {
            results: response
                .tool_calls
                .iter()
                .map(|call| Receipt {
                    call_id: call.id.clone(),
                    output: Output::not_executed("turn stopped").text,
                    summary: "Not executed".into(),
                    image: None,
                })
                .collect(),
            response: Some(response),
            accepted: true,
            ..Default::default()
        });
        history.checkpoint().unwrap();
        let workspace = Workspace::open(&files.0).unwrap();
        let (tx, rx) = mpsc::sync_channel(0);
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancelled);
        let runner = std::thread::spawn(move || {
            let context = worker::Context {
                events: tx,
                cancelled: flag,
                stopped: Arc::new(AtomicBool::new(false)),
                guidance: Arc::new(super::super::queue::Pending::default()),
                next_effect: AtomicU64::new(1),
                effect_gate: None,
            };
            let mut metrics = Metrics::default();
            let result = execute(
                &mut UnusedBackend,
                &mut history,
                &workspace,
                &context,
                Model::Luna,
                &Instant::now,
                &mut metrics,
            );
            (result, history, metrics)
        });
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Event::ToolStarted {
                name: "recall_receipts",
                ..
            }
        ));
        assert!(matches!(rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Event::ToolFinished { failed, .. } if failed != admitted));
        cancelled.store(true, Ordering::Release);
        let (result, mut history, metrics) = runner.join().unwrap();
        assert_eq!(result, Err(Failure::Cancelled));
        assert_eq!(metrics.tool_calls, 1);
        assert_eq!(history.turns[0].steps[1].results[1].summary, "Not executed");
        let recorded = history.turns[0].steps[1].results[0].output.clone();
        if !admitted {
            assert!(recorded.contains("expected_call_id does not match"));
            assert!(!recorded.contains("ORIGINAL-RECEIPT"));
        }
        assert_eq!(history.pending_recall_step(), admitted.then_some((0, 1)));
        history.turns[0].end = Some(End::Failed(Failure::Cancelled));
        history.turns[0].outcome = Failure::Cancelled.to_string();
        history.projection.limit_bytes = 65536;
        history.checkpoint().unwrap();
        drop(history);
        std::fs::remove_file(files.0.join("source.txt")).unwrap();
        let saved = super::super::persistence::load(&store, &id, true).unwrap();
        assert_eq!(
            saved.history.pending_recall_step(),
            admitted.then_some((0, 1))
        );
        let compactions = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut session = super::super::Session::with_history(
            Model::Luna,
            ResumeProbe {
                compactions: compactions.clone(),
                seen: seen.clone(),
            },
            Some(Workspace::open(&files.0).unwrap()),
            saved.history,
        )
        .unwrap();
        assert!(matches!(
            super::super::tests::next(&mut session),
            Event::Restored { .. }
        ));
        assert!(matches!(
            super::super::tests::next(&mut session),
            Event::Ready
        ));
        assert!(session.submit("Continue from the cancelled batch"));
        let mut replayed_tools = 0;
        loop {
            match super::super::tests::next(&mut session) {
                Event::ToolStarted { .. } => replayed_tools += 1,
                Event::Finished(end, _) => {
                    assert_eq!(end, End::Complete);
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(replayed_tools, 0);
        if admitted {
            assert_eq!(compactions.load(Ordering::Relaxed), 0);
            assert!(seen.lock().unwrap().contains(&("recall".into(), recorded)));
        } else {
            assert!(compactions.load(Ordering::Relaxed) > 0);
        }
    }
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn completed_read_in_cancelled_batch_survives_checkpoint_compaction_and_resume() {
    let files = crate::workspace_fixture::Fixture::new();
    let original = "old-observation-Ω-".repeat(300);
    files.write("a.txt", &original);
    files.write("b.txt", "sibling must not run");
    let fixture = crate::state::tests::Fixture::new();
    let store = fixture.store().unwrap();
    let mut history =
        super::super::persistence::create_in(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("Read a and b".into()).unwrap();
    let response = tool_tests::calls_response(vec![
        tool_tests::call("read-a", "read_file", r#"{"path":"a.txt"}"#),
        tool_tests::call("read-b", "read_file", r#"{"path":"b.txt"}"#),
    ]);
    history.turns[0].steps.push(Step {
        results: response
            .tool_calls
            .iter()
            .map(|call| Receipt {
                call_id: call.id.clone(),
                output: Output::error("tool was not executed because the turn stopped").text,
                summary: "Not executed".into(),
                image: None,
            })
            .collect(),
        response: Some(response),
        accepted: true,
        ..Default::default()
    });
    history.checkpoint().unwrap();
    let workspace = Workspace::open(&files.0).unwrap();
    let (tx, rx) = mpsc::sync_channel(0);
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancelled);
    let runner = std::thread::spawn(move || {
        let context = worker::Context {
            events: tx,
            cancelled: flag,
            stopped: Arc::new(AtomicBool::new(false)),
            guidance: Arc::new(super::super::queue::Pending::default()),
            next_effect: AtomicU64::new(1),
            effect_gate: None,
        };
        let mut metrics = Metrics::default();
        let result = execute(
            &mut UnusedBackend,
            &mut history,
            &workspace,
            &context,
            Model::Luna,
            &Instant::now,
            &mut metrics,
        );
        (result, history, metrics)
    });
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        Event::ToolStarted {
            name: "read_file",
            ..
        }
    ));
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        Event::ToolFinished { failed: false, .. }
    ));
    cancelled.store(true, Ordering::Release);
    let (result, mut history, metrics) = runner.join().unwrap();
    assert_eq!(result, Err(Failure::Cancelled));
    assert_eq!(metrics.tool_calls, 1);
    assert_ne!(history.turns[0].steps[0].results[0].summary, "Not executed");
    assert_eq!(history.turns[0].steps[0].results[1].summary, "Not executed");
    let recorded = history.turns[0].steps[0].results[0].output.clone();
    history.turns[0].end = Some(End::Failed(Failure::Cancelled));
    history.turns[0].outcome = Failure::Cancelled.to_string();
    history.checkpoint().unwrap();
    let (compact_tx, _compact_rx) = mpsc::sync_channel(64);
    let compact_context = worker::Context {
        events: compact_tx,
        cancelled: Arc::new(AtomicBool::new(false)),
        stopped: Arc::new(AtomicBool::new(false)),
        guidance: Arc::new(super::super::queue::Pending::default()),
        next_effect: AtomicU64::new(1),
        effect_gate: None,
    };
    let mut compact_metrics = Metrics::default();
    super::super::context::compact(
        &mut UnusedBackend,
        &mut history,
        &compact_context,
        Model::Luna,
        true,
        &mut compact_metrics,
    )
    .unwrap();
    assert_eq!(compact_metrics.requests, 1);
    assert!(history.base_turn == 1 || history.base_step == 1);
    drop(history);
    std::fs::remove_file(files.0.join("a.txt")).unwrap();
    std::fs::remove_file(files.0.join("b.txt")).unwrap();
    let saved = super::super::persistence::load(&store, &id, true).unwrap();
    assert!(saved.history.base_turn == 1 || saved.history.base_step == 1);
    let no_cancel = AtomicBool::new(false);
    let page = super::super::receipt_recall::execute(
        &saved.history,
        0,
        0,
        0,
        0,
        &Budget {
            cancelled: &no_cancel,
            deadline: Instant::now() + Duration::from_secs(10),
        },
    );
    assert!(!page.failed, "{}", page.text);
    let value = crate::json::parse(&page.text, Default::default()).unwrap();
    let output = value
        .get("output")
        .and_then(crate::json::Value::text)
        .unwrap();
    assert_eq!(output, recorded);
    assert!(output.contains("old-observation-Ω-"));
    assert!(
        value
            .get("next")
            .is_some_and(|next| matches!(next, crate::json::Value::Null))
    );
    let sibling = super::super::receipt_recall::execute(
        &saved.history,
        0,
        0,
        1,
        0,
        &Budget {
            cancelled: &no_cancel,
            deadline: Instant::now() + Duration::from_secs(10),
        },
    );
    assert!(sibling.failed);
}

#[test]
fn encoded_recall_admission_pairs_tail_without_running_effect() {
    let files = crate::workspace_fixture::Fixture::new();
    files.write("edit.txt", "old value");
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history = History::default();
    history.begin("Recover the saved read".into()).unwrap();
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(vec![tool_tests::call(
            "source",
            "read_file",
            r#"{"path":"source.txt"}"#,
        )])),
        results: vec![Receipt {
            call_id: "source".into(),
            output: "x".repeat(8000),
            summary: "read_file / source.txt".into(),
            image: None,
        }],
        accepted: true,
        ..Default::default()
    });
    let calls = tool_tests::calls_response(vec![
        tool_tests::call("recall", "recall_receipts", r#"{"turn":0,"step":0}"#),
        tool_tests::call(
            "edit",
            "edit_file",
            r#"{"path":"edit.txt","old_text":"old","new_text":"new"}"#,
        ),
    ]);
    history.turns[0].steps.push(Step {
        results: calls
            .tool_calls
            .iter()
            .map(|call| Receipt {
                call_id: call.id.clone(),
                output: Output::error("tool was not executed because the turn stopped").text,
                summary: "Not executed".into(),
                image: None,
            })
            .collect(),
        response: Some(calls),
        accepted: true,
        ..Default::default()
    });
    let base = history
        .unabridged_projected_request(Model::Luna, true)
        .unwrap()
        .encode(MAX_REQUEST)
        .unwrap()
        .len();
    history.projection.summary = "p".repeat(MAX_REQUEST - base - 4096);
    assert!(
        history
            .unabridged_projected_request(Model::Luna, true)
            .unwrap()
            .encode(MAX_REQUEST)
            .is_ok()
    );
    let (tx, _rx) = mpsc::sync_channel(64);
    let context = worker::Context {
        events: tx,
        cancelled: Arc::new(AtomicBool::new(false)),
        stopped: Arc::new(AtomicBool::new(false)),
        guidance: Arc::new(super::super::queue::Pending::default()),
        next_effect: AtomicU64::new(1),
        effect_gate: None,
    };
    let mut metrics = Metrics::default();
    execute(
        &mut UnusedBackend,
        &mut history,
        &workspace,
        &context,
        Model::Luna,
        &Instant::now,
        &mut metrics,
    )
    .unwrap();
    assert_eq!(metrics.tool_calls, 2);
    let results = &history.turns[0].steps[1].results;
    assert!(results[0].output.contains("not admitted"));
    assert!(results[1].output.contains("not_executed"));
    assert_eq!(
        std::fs::read_to_string(files.0.join("edit.txt")).unwrap(),
        "old value"
    );
    let request = history.request(Model::Luna, true).unwrap();
    let encoded = request.encode(MAX_REQUEST).unwrap();
    assert!(encoded.contains("recall result was not admitted"));
    assert!(encoded.contains("tool was not executed because the aggregate"));
}
