use super::*;
use crate::{
    json::{self, Value},
    providers::openai_account::{Progress, Request, Response, Status, client},
    session::{Session, tool_tests, worker},
    tls::Budget as NetworkBudget,
    workspace::Workspace,
};
use std::{
    ops::ControlFlow,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

struct NoReplay(Arc<AtomicUsize>);
impl Backend for NoReplay {
    fn login(
        &mut self,
        _: &NetworkBudget<'_>,
        _: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        Ok(())
    }
    fn generate(
        &mut self,
        _: &Request,
        _: &NetworkBudget<'_>,
        _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(crate::session::tests::response(
            "Continue",
            Status::Completed,
        ))
    }
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn cancelled_131_call_batch_resumes_and_recalls_completed_prefix_only() {
    const COMPLETED: usize = 129;
    let files = crate::workspace_fixture::Fixture::new();
    for n in 0..COMPLETED {
        files.write(&format!("file-{n}.txt"), format!("ORIGINAL-{n:03}\n"));
    }
    let fixture = crate::state::tests::Fixture::new();
    let store = fixture.store().unwrap();
    let mut history =
        super::super::persistence::create_in(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("Read a cancellable batch".into()).unwrap();
    let calls = (0..COMPLETED + 2)
        .map(|n| {
            tool_tests::call(
                &format!("read-{n}"),
                "read_file",
                &format!(r#"{{"path":"file-{n}.txt"}}"#),
            )
        })
        .collect::<Vec<_>>();
    let response = tool_tests::calls_response(calls);
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
            &mut NoReplay(Arc::new(AtomicUsize::new(0))),
            &mut history,
            &workspace,
            &context,
            Model::Luna,
            &Instant::now,
            &mut metrics,
        );
        (result, history, metrics)
    });
    let mut completed = 0;
    while completed < COMPLETED {
        match rx.recv_timeout(Duration::from_secs(20)).unwrap() {
            Event::ToolStarted {
                name: "read_file", ..
            } => {}
            Event::ToolFinished { failed: false, .. } => completed += 1,
            _ => panic!("unexpected batch event"),
        }
    }
    cancelled.store(true, Ordering::Release);
    let (result, mut history, metrics) = runner.join().unwrap();
    assert_eq!(result, Err(Failure::Cancelled));
    assert_eq!(metrics.tool_calls, COMPLETED as u32);
    let step = &history.turns[0].steps[0];
    assert_eq!(step.results.len(), COMPLETED + 2);
    assert_eq!(step.results[COMPLETED].summary, "Not executed");
    assert_eq!(step.results[COMPLETED + 1].summary, "Not executed");
    let expected = [0, 64, 128].map(|n| step.results[n].output.clone());
    // An interrupted read with an uncertain outcome is also outside recall's
    // completed-observation scope. Preserve the following unexecuted sibling.
    history.turns[0].steps[0].results[COMPLETED].output = json::encode(
        &json::object([
            ("ok", Value::Bool(false)),
            ("status", Value::String("uncertain".into())),
        ]),
        crate::tools::MAX_OUTPUT,
    )
    .unwrap();
    history.turns[0].steps[0].results[COMPLETED].summary = "read_file / outcome unknown".into();
    history.turns[0].end = Some(End::Failed(Failure::Cancelled));
    history.turns[0].outcome = Failure::Cancelled.to_string();
    history.checkpoint().unwrap();
    drop(history);
    for n in 0..COMPLETED {
        std::fs::remove_file(files.0.join(format!("file-{n}.txt"))).unwrap();
    }
    let mut saved = super::super::persistence::load(&store, &id, true).unwrap();
    assert_eq!(saved.history.turns[0].steps[0].results.len(), COMPLETED + 2);
    saved.history.projection.through = 1;
    saved.history.projection.summary =
        "Completed read prefix remains in canonical receipts.".into();
    saved.history.checkpoint().unwrap();
    saved.history.release_projected();
    drop(saved);
    let saved = super::super::persistence::load(&store, &id, true).unwrap();
    let no_cancel = AtomicBool::new(false);
    for (index, n) in [0, 64, 128].iter().enumerate() {
        let page = super::super::receipt_recall::execute(
            &saved.history,
            0,
            0,
            *n,
            0,
            &crate::workspace::Budget {
                cancelled: &no_cancel,
                deadline: Instant::now() + Duration::from_secs(10),
            },
        );
        assert!(!page.failed, "{}", page.text);
        let value = json::parse(&page.text, Default::default()).unwrap();
        assert_eq!(
            value.get("call_id").and_then(Value::text),
            Some(format!("read-{n}").as_str())
        );
        assert_eq!(
            value.get("output").and_then(Value::text),
            Some(expected[index].as_str())
        );
    }
    for n in [COMPLETED, COMPLETED + 1] {
        let denied = super::super::receipt_recall::execute(
            &saved.history,
            0,
            0,
            n,
            0,
            &crate::workspace::Budget {
                cancelled: &no_cancel,
                deadline: Instant::now() + Duration::from_secs(10),
            },
        );
        assert!(denied.failed);
    }
    let generated = Arc::new(AtomicUsize::new(0));
    let mut session = Session::with_history(
        Model::Luna,
        NoReplay(generated.clone()),
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
    assert!(session.submit("Continue after cancellation"));
    loop {
        match super::super::tests::next(&mut session) {
            Event::ToolStarted { .. } => panic!("historical tool replayed"),
            Event::Finished(end, metrics) => {
                assert_eq!(end, End::Complete);
                assert_eq!(metrics.tool_calls, 0);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(generated.load(Ordering::Relaxed), 1);
}
