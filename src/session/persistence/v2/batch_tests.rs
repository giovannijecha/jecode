use super::*;
use crate::{
    json::{self, Value},
    providers::openai_account::{Input, Progress, Request, Response, Status, client},
    session::{End, Event, Session, tool_tests, worker::Backend},
    tls::Budget as NetworkBudget,
    workspace::{Budget, Workspace},
};
use std::{
    ops::ControlFlow,
    sync::{Arc, Mutex, atomic::AtomicBool},
    thread,
    time::{Duration, Instant},
};

const CALLS: usize = 129;
const SAMPLE: [usize; 3] = [0, 64, 128];

fn next(session: &mut Session) -> Event {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(event) = session.poll() {
            return event;
        }
        assert!(Instant::now() < deadline, "persisted batch worker stalled");
        thread::sleep(Duration::from_millis(1));
    }
}

struct ReadBatch {
    round: usize,
}
impl Backend for ReadBatch {
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
        self.round += 1;
        if self.round == 1 {
            let calls = (0..CALLS)
                .map(|n| {
                    tool_tests::call(
                        &format!("read-{n}"),
                        "read_file",
                        &format!(r#"{{"path":"file-{n}.txt"}}"#),
                    )
                })
                .collect();
            return Ok(tool_tests::calls_response(calls));
        }
        Ok(crate::session::tests::response(
            "Read batch complete",
            Status::Completed,
        ))
    }
}

struct RecallBatch {
    round: usize,
    expected: Arc<Vec<String>>,
    observed: Arc<Mutex<usize>>,
}
impl Backend for RecallBatch {
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
        self.round += 1;
        if self.round == 1 {
            let calls = SAMPLE
                .iter()
                .map(|n| {
                    tool_tests::call(
                        &format!("recall-{n}"),
                        "recall_receipts",
                        &format!(r#"{{"turn":0,"step":0,"receipt":{n}}}"#),
                    )
                })
                .collect();
            return Ok(tool_tests::calls_response(calls));
        }
        let receipts = request
            .input
            .iter()
            .filter_map(|item| {
                if let Input::ToolResult { call_id, output } = item
                    && call_id.starts_with("recall-")
                {
                    Some((call_id, output))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(receipts.len(), SAMPLE.len());
        for (index, (call_id, output)) in receipts.iter().enumerate() {
            let n = SAMPLE[index];
            assert_eq!(*call_id, &format!("recall-{n}"));
            let page = json::parse(output, Default::default()).unwrap();
            assert_eq!(page.get("ok"), Some(&Value::Bool(true)));
            assert_eq!(
                page.get("receipt").and_then(Value::unsigned),
                Some(n as u64)
            );
            assert_eq!(
                page.get("call_id").and_then(Value::text),
                Some(format!("read-{n}").as_str())
            );
            assert_eq!(
                page.get("output").and_then(Value::text),
                Some(self.expected[index].as_str())
            );
        }
        *self.observed.lock().unwrap() = receipts.len();
        Ok(crate::session::tests::response(
            "Original receipts recovered",
            Status::Completed,
        ))
    }
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn executed_129_read_batch_survives_resume_release_and_model_recall() {
    let files = crate::workspace_fixture::Fixture::new();
    for n in 0..CALLS {
        files.write(&format!("file-{n}.txt"), format!("ORIGINAL-{n:03}\n"));
    }
    let fixture = crate::state::tests::Fixture::new();
    let store = fixture.store().unwrap();
    let workspace = Workspace::open(&files.0).unwrap();
    let history = create(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    let mut session = Session::with_history(
        Model::Luna,
        ReadBatch { round: 0 },
        Some(workspace),
        history,
    )
    .unwrap();
    assert!(matches!(next(&mut session), Event::Restored { .. }));
    assert!(matches!(next(&mut session), Event::Ready));
    assert!(session.submit("Read all 129 files"));
    let mut finished_tools = 0;
    loop {
        match next(&mut session) {
            Event::ToolFinished { failed, .. } => {
                assert!(!failed);
                finished_tools += 1;
            }
            Event::Finished(end, metrics) => {
                assert_eq!(end, End::Complete);
                assert_eq!(metrics.tool_calls, CALLS as u32);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(finished_tools, CALLS);
    drop(session);
    for n in 0..CALLS {
        std::fs::remove_file(files.0.join(format!("file-{n}.txt"))).unwrap();
    }
    let mut saved = super::super::load(&store, &id, true).unwrap();
    let step = &saved.history.turns[0].steps[0];
    assert_eq!(step.results.len(), CALLS);
    let mut expected = Vec::new();
    for n in SAMPLE {
        assert_eq!(
            step.response.as_ref().unwrap().tool_calls[n].id,
            format!("read-{n}")
        );
        assert_eq!(step.results[n].call_id, format!("read-{n}"));
        let value = json::parse(&step.results[n].output, Default::default()).unwrap();
        assert_eq!(
            value.get("text").and_then(Value::text),
            Some(format!("ORIGINAL-{n:03}\n").as_str())
        );
        expected.push(step.results[n].output.clone());
    }
    saved.history.projection.through = 1;
    saved.history.projection.summary =
        "Original 129 reads completed; receipts remain available.".into();
    saved.history.checkpoint().unwrap();
    saved.history.release_projected();
    drop(saved);
    let saved = super::super::load(&store, &id, true).unwrap();
    assert_eq!(saved.history.base_turn, 1);
    let cancelled = AtomicBool::new(false);
    for (index, n) in SAMPLE.iter().enumerate() {
        let page = crate::session::receipt_recall::execute(
            &saved.history,
            0,
            0,
            *n,
            0,
            &Budget {
                cancelled: &cancelled,
                deadline: Instant::now() + Duration::from_secs(10),
            },
        );
        assert!(!page.failed, "{}", page.text);
        let value = json::parse(&page.text, Default::default()).unwrap();
        assert_eq!(
            value.get("output").and_then(Value::text),
            Some(expected[index].as_str())
        );
    }
    let observed = Arc::new(Mutex::new(0));
    let mut session = Session::with_history(
        Model::Luna,
        RecallBatch {
            round: 0,
            expected: Arc::new(expected),
            observed: observed.clone(),
        },
        Some(Workspace::open(&files.0).unwrap()),
        saved.history,
    )
    .unwrap();
    assert!(matches!(next(&mut session), Event::Restored { .. }));
    assert!(matches!(next(&mut session), Event::Ready));
    assert!(session.submit("Recall the first, middle and final original reads"));
    let mut tools = 0;
    loop {
        match next(&mut session) {
            Event::ToolStarted {
                name: "recall_receipts",
                ..
            } => tools += 1,
            Event::ToolStarted { name, .. } => panic!("historical tool replayed: {name}"),
            Event::Finished(end, metrics) => {
                assert_eq!(end, End::Complete);
                assert_eq!(metrics.tool_calls, SAMPLE.len() as u32);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(tools, SAMPLE.len());
    assert_eq!(*observed.lock().unwrap(), SAMPLE.len());
}
