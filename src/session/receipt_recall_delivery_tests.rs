use super::*;
use crate::{
    providers::openai_account::{Input, Progress, Request, Response, Status, client},
    session::{End, Event, Model, Session, history::Receipt, tool_tests, worker::Backend},
    tls::Budget as NetworkBudget,
    workspace::Workspace,
};
use std::{
    ops::ControlFlow,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Default)]
struct Observed {
    compactions: AtomicUsize,
    generations: Mutex<Vec<Vec<(String, String)>>>,
}
struct ObserveBackend(Arc<Observed>);
impl Backend for ObserveBackend {
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
            self.0.compactions.fetch_add(1, Ordering::Relaxed);
            return Ok(crate::session::tests::handoff_response(
                request,
                "The recorded reads and their outcomes remain in canonical receipts.",
            ));
        }
        assert!(request.encode(super::super::history::MAX_REQUEST).is_ok());
        self.0.generations.lock().unwrap().push(
            request
                .input
                .iter()
                .filter_map(|item| match item {
                    Input::ToolResult { call_id, output } => {
                        Some((call_id.clone(), output.clone()))
                    }
                    _ => None,
                })
                .collect(),
        );
        Ok(crate::session::tests::response(
            "Evidence received",
            Status::Completed,
        ))
    }
}

#[derive(Clone, Copy)]
enum RecallOutcome {
    Failed,
    Rejected,
    Unexecuted,
    Admitted,
}
fn mixed_history(position: usize, outcome: RecallOutcome) -> (History, String) {
    let mut history = History::default();
    history.begin("Read the source".into()).unwrap();
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(vec![tool_tests::call(
            "source",
            "read_file",
            r#"{"path":"source.txt"}"#,
        )])),
        results: vec![Receipt {
            call_id: "source".into(),
            output: "original observation ".repeat(1000),
            summary: "read_file / source.txt".into(),
            image: None,
        }],
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Complete);
    let cancelled = AtomicBool::new(false);
    let page = execute(
        &history,
        0,
        0,
        0,
        0,
        &Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(10),
        },
    );
    assert!(!page.failed);
    history.begin("Mixed recorded calls".into()).unwrap();
    let mut calls = Vec::new();
    let mut results = Vec::new();
    for index in 0..3 {
        if index == position {
            calls.push(tool_tests::call(
                "recall",
                "recall_receipts",
                r#"{"turn":0,"step":0}"#,
            ));
            let (output, summary) = match outcome {
                RecallOutcome::Failed => (
                    Output::error("requested receipt unavailable").text,
                    "recall_receipts / failed".into(),
                ),
                RecallOutcome::Rejected => (
                    Output::error("aggregate delivery rejected this result").text,
                    "recall_receipts / rejected".into(),
                ),
                RecallOutcome::Unexecuted => (
                    Output::not_executed("batch ended").text,
                    "Not executed".into(),
                ),
                RecallOutcome::Admitted => (
                    page.text.clone(),
                    "recall_receipts / recorded evidence".into(),
                ),
            };
            results.push(Receipt {
                call_id: "recall".into(),
                output,
                summary,
                image: None,
            });
        } else {
            let id = format!("read-{index}");
            calls.push(tool_tests::call(
                &id,
                "read_file",
                r#"{"path":"source.txt"}"#,
            ));
            results.push(Receipt {
                call_id: id,
                output: "mixed original read ".repeat(1000),
                summary: "read_file / source.txt".into(),
                image: None,
            });
        }
    }
    history.turns[1].steps.push(Step {
        response: Some(tool_tests::calls_response(calls)),
        results,
        accepted: true,
        ..Default::default()
    });
    history.turns[1].end = Some(End::Complete);
    history.projection.limit_bytes = 1;
    (history, page.text)
}

fn complete(session: &mut Session) -> usize {
    assert!(session.submit("Continue from the recorded outcomes"));
    let mut historical_tools = 0;
    loop {
        match super::review_tests::next_large(session) {
            Event::ToolStarted { .. } => historical_tools += 1,
            Event::Finished(end, _) => {
                assert_eq!(end, End::Complete);
                return historical_tools;
            }
            _ => {}
        }
    }
}

#[test]
fn failed_and_unexecuted_recalls_do_not_pin_mixed_reads_at_any_position() {
    let files = crate::workspace_fixture::Fixture::new();
    for position in 0..3 {
        for outcome in [
            RecallOutcome::Failed,
            RecallOutcome::Rejected,
            RecallOutcome::Unexecuted,
        ] {
            let (history, _) = mixed_history(position, outcome);
            assert_eq!(history.pending_recall_step(), None);
            let observed = Arc::new(Observed::default());
            let mut session = Session::with_history(
                Model::Luna,
                ObserveBackend(observed.clone()),
                Some(Workspace::open(&files.0).unwrap()),
                history,
            )
            .unwrap();
            assert!(matches!(
                super::review_tests::next_large(&mut session),
                Event::Ready
            ));
            assert_eq!(complete(&mut session), 0);
            assert!(observed.compactions.load(Ordering::Relaxed) > 0);
            assert_eq!(observed.generations.lock().unwrap().len(), 1);
        }
    }
}

#[test]
fn admitted_recall_in_mixed_reads_is_delivered_then_can_be_compacted() {
    let files = crate::workspace_fixture::Fixture::new();
    for position in 0..3 {
        let (history, page) = mixed_history(position, RecallOutcome::Admitted);
        assert_eq!(history.pending_recall_step(), Some((1, 0)));
        let observed = Arc::new(Observed::default());
        let mut session = Session::with_history(
            Model::Luna,
            ObserveBackend(observed.clone()),
            Some(Workspace::open(&files.0).unwrap()),
            history,
        )
        .unwrap();
        assert!(matches!(
            super::review_tests::next_large(&mut session),
            Event::Ready
        ));
        assert_eq!(complete(&mut session), 0);
        assert_eq!(observed.compactions.load(Ordering::Relaxed), 0);
        let generations = observed.generations.lock().unwrap();
        let receipts = &generations[0];
        assert_eq!(receipts.len(), 4);
        assert_eq!(receipts[position + 1], ("recall".into(), page));
        drop(generations);
        assert_eq!(complete(&mut session), 0);
        assert!(observed.compactions.load(Ordering::Relaxed) > 0);
    }
}
