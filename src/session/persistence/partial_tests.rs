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
            Ok(session::tests::handoff_response(
                request,
                if self.fail_with.is_some() {
                    "First partial slice preserved the task and completed receipts."
                } else {
                    "All completed receipts were summarized; no effect should repeat."
                },
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
                image: None,
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
    let context = session::worker::Context {
        events,
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        stopped: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        guidance: Arc::new(session::queue::Pending::default()),
        next_effect: std::sync::atomic::AtomicU64::new(1),
        effect_gate: None,
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

#[test]
fn one_oversized_call_and_receipt_resume_across_encoded_reference_slices() {
    struct Sliced {
        calls: usize,
        fail_at: Option<(usize, Failure)>,
        seen: Arc<Mutex<Vec<String>>>,
    }
    impl session::worker::Backend for Sliced {
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
            assert!(request.tools.is_empty());
            let encoded = request.encode(session::history::MAX_CONTEXT)?;
            self.seen.lock().unwrap().push(encoded);
            self.calls += 1;
            if let Some((at, cause)) = self.fail_at
                && at == self.calls
            {
                return match cause {
                    Failure::Cancelled => Err(crate::tls::NetworkError::Cancelled.into()),
                    Failure::Account(client::Error::Expired) => Err(client::Error::Expired),
                    Failure::CompactionOutput => {
                        Ok(session::tests::response("", Status::Completed))
                    }
                    _ => unreachable!(),
                };
            }
            Ok(session::tests::handoff_response(
                request,
                &format!(
                    "Covered ordered reference slice {} with exact call and receipt association.",
                    self.calls
                ),
            ))
        }
    }
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create_in(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history
        .begin("Finish the task after the large read".into())
        .unwrap();
    let call = session::tool_tests::call("oversized-call", "read_file", "{\"path\":\"big.txt\"}");
    let output = format!(
        "FIRST-MARKER{}MIDDLE-MARKER{}LAST-MARKER",
        "\u{0001}".repeat(320_000),
        "\u{0001}".repeat(320_000)
    );
    assert!(output.len() < 1024 * 1024);
    history.turns[0].steps.push(Step {
        response: Some(session::tool_tests::calls_response(vec![call])),
        accepted: true,
        results: vec![Receipt {
            call_id: "oversized-call".into(),
            output: output.clone(),
            summary: "Read big.txt".into(),
            image: None,
        }],
        ..Default::default()
    });
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    history.checkpoint().unwrap();
    let encoded_receipt_bytes = crate::json::encode(
        &crate::json::Value::String(output.clone()),
        80 * 1024 * 1024,
    )
    .unwrap()
    .len();
    assert!(encoded_receipt_bytes > 1024 * 1024);
    let sizes = session::context::partial::reference_sizes(&history).unwrap();
    let record_bytes = *sizes.iter().max().unwrap();
    let call_record = sizes.iter().position(|size| *size == record_bytes).unwrap();
    assert!(record_bytes > session::history::MAX_CONTEXT);
    let (events, _received) = std::sync::mpsc::sync_channel(64);
    let context = session::worker::Context {
        events,
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        stopped: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        guidance: Arc::new(session::queue::Pending::default()),
        next_effect: std::sync::atomic::AtomicU64::new(1),
        effect_gate: None,
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut first = Sliced {
        calls: 0,
        fail_at: Some((2, Failure::Cancelled)),
        seen: seen.clone(),
    };
    let mut metrics = session::Metrics::default();
    assert_eq!(
        session::context::compact(
            &mut first,
            &mut history,
            &context,
            Model::Luna,
            false,
            &mut metrics
        ),
        Err(Failure::Cancelled)
    );
    assert_eq!(history.projection.through, 0);
    assert_eq!(history.projection.step, 0);
    assert!(history.projection.pending.is_some());
    drop(history);
    let mut saved = load(&store, &id, true).unwrap();
    assert!(saved.history.projection.pending.is_some());
    let prior = saved
        .history
        .projection
        .pending
        .as_ref()
        .unwrap()
        .summary
        .clone();
    for cause in [
        Failure::CompactionOutput,
        Failure::Account(client::Error::Expired),
    ] {
        let mut invalid = Sliced {
            calls: 0,
            fail_at: Some((1, cause)),
            seen: seen.clone(),
        };
        assert_eq!(
            session::context::compact(
                &mut invalid,
                &mut saved.history,
                &context,
                Model::Luna,
                false,
                &mut metrics
            ),
            Err(cause)
        );
        assert_eq!(
            saved.history.projection.pending.as_ref().unwrap().summary,
            prior
        );
        assert_eq!(saved.history.projection.step, 0);
    }
    saved
        .history
        .fail_next_checkpoint
        .store(true, std::sync::atomic::Ordering::Release);
    let mut storage = Sliced {
        calls: 0,
        fail_at: None,
        seen: seen.clone(),
    };
    assert_eq!(
        session::context::compact(
            &mut storage,
            &mut saved.history,
            &context,
            Model::Luna,
            false,
            &mut metrics
        ),
        Err(Failure::Storage)
    );
    assert_eq!(
        saved.history.projection.pending.as_ref().unwrap().summary,
        prior
    );
    let mut resumed = Sliced {
        calls: 0,
        fail_at: None,
        seen: seen.clone(),
    };
    assert_eq!(
        session::context::compact(
            &mut resumed,
            &mut saved.history,
            &context,
            Model::Luna,
            false,
            &mut metrics
        ),
        Ok(())
    );
    assert!(saved.history.projection.pending.is_none());
    assert_eq!(saved.history.projection.step, 0); // Completed turn was released from memory.
    assert_eq!(saved.history.turn_count(), 1);
    let requests = seen.lock().unwrap();
    let max_encoded = requests.iter().map(String::len).max().unwrap();
    eprintln!(
        "oversized reference: record_bytes={record_bytes} encoded_receipt_bytes={encoded_receipt_bytes} requests={} max_encoded_request_bytes={max_encoded}",
        requests.len()
    );
    assert!(requests.len() >= 3);
    assert!(max_encoded <= session::history::MAX_CONTEXT);
    // Cancelled, invalid, unauthenticated and uncheckpointed requests cannot
    // advance validated coverage; their reference bytes must be sent again.
    for marker in ["FIRST-MARKER", "MIDDLE-MARKER", "LAST-MARKER"] {
        assert_eq!(
            requests
                .iter()
                .enumerate()
                .filter(|(index, _)| *index == 0 || *index >= 5)
                .map(|(_, request)| request.matches(marker).count())
                .sum::<usize>(),
            1,
            "{marker} was repeated or omitted in validated coverage"
        );
    }
    let mut reconstructed = String::new();
    let mut slices = 0;
    for (_, request) in requests
        .iter()
        .enumerate()
        .filter(|(index, _)| *index == 0 || *index >= 5)
    {
        let value = crate::json::parse(
            request,
            crate::json::Limits {
                bytes: session::history::MAX_CONTEXT,
                ..Default::default()
            },
        )
        .unwrap();
        for item in value
            .get("input")
            .and_then(crate::json::Value::array)
            .unwrap()
        {
            let Some(data) = item
                .get("content")
                .and_then(crate::json::Value::array)
                .and_then(|parts| parts.first())
                .and_then(|part| part.get("text"))
                .and_then(crate::json::Value::text)
            else {
                continue;
            };
            let Some((header, fragment)) = data.split_once('\n') else {
                continue;
            };
            if !header.starts_with(&format!("Completed step record {call_record},")) {
                continue;
            }
            assert!(header.contains("call_id=oversized-call"));
            assert!(header.contains("receipt_call_id=oversized-call"));
            let range = header
                .split("bytes ")
                .nth(1)
                .unwrap()
                .split(';')
                .next()
                .unwrap();
            let (start, rest) = range.split_once("..").unwrap();
            let (end, total) = rest.split_once(" of ").unwrap();
            assert_eq!(start.parse::<usize>().unwrap(), reconstructed.len());
            assert_eq!(total.parse::<usize>().unwrap(), record_bytes);
            reconstructed.push_str(fragment);
            assert_eq!(end.parse::<usize>().unwrap(), reconstructed.len());
            slices += 1;
        }
    }
    assert!(slices >= 2);
    assert_eq!(reconstructed.len(), record_bytes);
    let covered = crate::json::parse(
        &reconstructed,
        crate::json::Limits {
            bytes: 80 * 1024 * 1024,
            nodes: 500_000,
            depth: 64,
        },
    )
    .unwrap();
    assert_eq!(
        covered.get("call_id").and_then(crate::json::Value::text),
        Some("oversized-call")
    );
    assert_eq!(
        covered
            .get("receipt_call_id")
            .and_then(crate::json::Value::text),
        Some("oversized-call")
    );
    assert_eq!(
        covered
            .get("parsed_arguments")
            .and_then(|v| v.get("path"))
            .and_then(crate::json::Value::text),
        Some("big.txt")
    );
    assert_eq!(
        covered
            .get("receipt_output")
            .and_then(crate::json::Value::text),
        Some(output.as_str())
    );
    drop(requests);
    let exact = super::v2::page(saved.history.record.as_ref().unwrap(), 0, 1).unwrap();
    assert_eq!(exact[0].steps[0].results[0].output, output);
    assert_eq!(exact[0].steps[0].results[0].call_id, "oversized-call");
}
