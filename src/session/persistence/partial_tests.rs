use super::*;
use crate::{
    providers::openai_account::{Progress, Request, Response, Status, client},
    session::{
        self, End, Failure,
        history::{Receipt, Step},
    },
    tls::Budget,
};
use std::{
    ops::ControlFlow,
    sync::{Arc, Mutex},
};

#[test]
fn partial_completed_step_checkpoint_resumes_without_replaying_receipts() {
    struct Slices {
        calls: usize,
        fail_at: usize,
        fail_with: Option<Failure>,
        observed: Arc<Mutex<Vec<String>>>,
    }
    impl session::worker::Backend for Slices {
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
            progress: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
        ) -> Result<Response, client::Error> {
            assert!(request.instructions.starts_with("Summarize"));
            assert!(request.tools.is_empty());
            self.observed
                .lock()
                .unwrap()
                .push(request.encode(session::history::MAX_CONTEXT)?);
            self.calls += 1;
            if self.calls == self.fail_at {
                match self.fail_with {
                    Some(Failure::Cancelled) => {
                        assert!(
                            progress(Progress::Text("unvalidated partial summary")).is_continue()
                        );
                        assert!(
                            progress(Progress::Attempt(client::Attempt {
                                delivery: client::Delivery::Streaming,
                                stage: Some(client::RequestStage::ResponseRead),
                                diagnostic: Some("synthetic interrupted compaction".into()),
                                ..Default::default()
                            }))
                            .is_continue()
                        );
                        return Err(crate::tls::NetworkError::Cancelled.into());
                    }
                    Some(Failure::Account(client::Error::Expired)) => {
                        return Err(client::Error::Expired);
                    }
                    Some(Failure::CompactionOutput) => {
                        return Ok(session::tests::response("", Status::Completed));
                    }
                    _ => {}
                }
            }
            Ok(session::tests::response(
                if self.fail_with.is_some() {
                    "First partial slice preserved the task and completed receipts."
                } else {
                    "All completed receipts were summarized; no effect should repeat."
                },
                Status::Completed,
            ))
        }
    }
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, None).unwrap();
    let id = history.record.as_ref().unwrap().id.clone();
    history
        .begin("Complete the original 100-part task".into())
        .unwrap();
    let calls = (0..100)
        .map(|n| {
            session::tool_tests::call(
                &format!("read-{n:03}"),
                "read_file",
                &format!(r#"{{"path":"part-{n:03}.txt"}}"#),
            )
        })
        .collect();
    history.turns[0].steps.push(Step {
        response: Some(session::tool_tests::calls_response(calls)),
        accepted: true,
        results: (0..100)
            .map(|n| Receipt {
                call_id: format!("read-{n:03}"),
                output: format!("MARKER-{n:03} {}", "x".repeat(25_000)),
                summary: format!("Read part-{n:03}.txt"),
            })
            .collect(),
        ..Default::default()
    });
    history.turns[0].guidance.push(session::queue::Guidance {
        after_step: 1,
        text: "GUIDANCE-AFTER-LARGE-STEP".into(),
    });
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    history.checkpoint().unwrap();
    let canonical = codec::encode(&history);
    let (events, _received) = std::sync::mpsc::sync_channel(64);
    let (_decisions, decisions) = std::sync::mpsc::sync_channel(1);
    let context = session::worker::Context {
        events,
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        stopped: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        decisions,
        guidance: Arc::new(session::queue::Pending::default()),
        next_approval: std::sync::atomic::AtomicU64::new(1),
    };
    let first = Arc::new(Mutex::new(Vec::new()));
    let mut backend = Slices {
        calls: 0,
        fail_at: 2,
        fail_with: Some(Failure::Cancelled),
        observed: first.clone(),
    };
    let mut metrics = session::Metrics::default();
    assert_eq!(
        session::context::compact(
            &mut backend,
            &mut history,
            &context,
            Model::Luna,
            false,
            &mut metrics
        ),
        Err(Failure::Cancelled)
    );
    assert_eq!(first.lock().unwrap().len(), 2);
    assert_eq!(codec::encode(&history), canonical);
    assert_eq!(
        (history.projection.through, history.projection.step),
        (0, 0)
    );
    assert!(history.projection.pending.is_some());
    assert_eq!(
        history.projection.failed_partial,
        "unvalidated partial summary"
    );
    assert_eq!(history.projection.failed_attempts.len(), 1);
    assert_eq!(
        history.projection.failed_attempts[0].delivery,
        client::Delivery::Streaming
    );
    let disk = store
        .directory("sessions")
        .unwrap()
        .read(&format!("{id}.json"), 16 * 1024 * 1024)
        .unwrap()
        .unwrap();
    assert!(disk.contains("unvalidated partial summary"));
    assert!(disk.contains("streaming_unvalidated"));
    for failure in [
        Failure::CompactionOutput,
        Failure::Account(client::Error::Expired),
    ] {
        let prior = history.projection.pending.as_ref().unwrap().summary.clone();
        let mut backend = Slices {
            calls: 0,
            fail_at: 1,
            fail_with: Some(failure),
            observed: Arc::new(Mutex::new(Vec::new())),
        };
        assert_eq!(
            session::context::compact(
                &mut backend,
                &mut history,
                &context,
                Model::Luna,
                false,
                &mut metrics
            ),
            Err(failure)
        );
        assert_eq!(history.projection.pending.as_ref().unwrap().summary, prior);
        assert_eq!(codec::encode(&history), canonical);
    }
    history
        .fail_next_checkpoint
        .store(true, std::sync::atomic::Ordering::Release);
    let prior = history.projection.pending.as_ref().unwrap().summary.clone();
    let mut backend = Slices {
        calls: 0,
        fail_at: 0,
        fail_with: None,
        observed: Arc::new(Mutex::new(Vec::new())),
    };
    assert_eq!(
        session::context::compact(
            &mut backend,
            &mut history,
            &context,
            Model::Luna,
            false,
            &mut metrics
        ),
        Err(Failure::Storage)
    );
    assert_eq!(history.projection.pending.as_ref().unwrap().summary, prior);
    assert_eq!(codec::encode(&history), canonical);
    drop(history);
    let mut saved = load(&store, &id, true).unwrap();
    assert_eq!(codec::encode(&saved.history), canonical);
    assert!(saved.history.projection.pending.is_some());
    assert!(saved.history.projection.failed);
    let resumed = Arc::new(Mutex::new(Vec::new()));
    let mut backend = Slices {
        calls: 0,
        fail_at: 0,
        fail_with: None,
        observed: resumed.clone(),
    };
    assert_eq!(
        session::context::compact(
            &mut backend,
            &mut saved.history,
            &context,
            Model::Luna,
            false,
            &mut metrics
        ),
        Ok(())
    );
    assert!(resumed.lock().unwrap()[0].contains("First partial slice preserved"));
    assert!(
        first
            .lock()
            .unwrap()
            .iter()
            .all(|request| !request.contains("GUIDANCE-AFTER-LARGE-STEP"))
    );
    assert!(
        resumed
            .lock()
            .unwrap()
            .iter()
            .all(|request| !request.contains("GUIDANCE-AFTER-LARGE-STEP"))
    );
    let mut markers = [0u8; 100];
    for encoded in std::iter::once(first.lock().unwrap()[0].clone())
        .chain(resumed.lock().unwrap().iter().cloned())
    {
        let request = crate::json::parse(&encoded, Default::default()).unwrap();
        for item in request
            .get("input")
            .and_then(crate::json::Value::array)
            .unwrap()
        {
            let Some(text) = item
                .get("content")
                .and_then(crate::json::Value::array)
                .and_then(|parts| parts.first())
                .and_then(|part| part.get("text"))
                .and_then(crate::json::Value::text)
            else {
                continue;
            };
            if !text.starts_with("Completed step record ") {
                continue;
            }
            for (n, count) in markers.iter_mut().enumerate() {
                if text.contains(&format!("MARKER-{n:03}")) {
                    *count += 1;
                }
            }
        }
    }
    assert!(
        markers.iter().all(|count| *count == 1),
        "partial checkpoint repeated or omitted a receipt"
    );
    assert!(saved.history.projection.pending.is_none());
    assert_eq!(saved.history.projection.step, 1);
    assert_eq!(codec::encode(&saved.history), canonical);
    let request = saved
        .history
        .request(Model::Luna, false)
        .unwrap()
        .encode(session::history::MAX_CONTEXT)
        .unwrap();
    assert!(request.contains("original 100-part task"));
    assert_eq!(request.matches("GUIDANCE-AFTER-LARGE-STEP").count(), 1);
    assert!(!request.contains("function_call_output"));
    assert!(!request.contains("read-000"));
}
