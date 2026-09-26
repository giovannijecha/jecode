use super::*;
use crate::{
    providers::openai_account::{Input, Progress, Response, client},
    session::{
        self, End,
        history::{Receipt, Step},
        worker::Backend,
    },
};
use std::{
    ops::ControlFlow,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64},
    },
};

const LEGACY_RECORD_BYTES: usize = 2_380_403;
const LEGACY_RECEIPT_START: usize = 368;
const LEGACY_REFERENCE_SHA256: &str =
    "40b33da7706eb58eeb325b2f43763377e56e0a818e205729e1e6bd32d78dec90";

struct Slices {
    accepted: Arc<Mutex<Vec<String>>>,
    cancel_at: Option<usize>,
    calls: usize,
}

impl Backend for Slices {
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
        self.calls += 1;
        assert!(request.tools.is_empty(), "compaction must not replay tools");
        if self.cancel_at == Some(self.calls) {
            return Err(crate::tls::NetworkError::Cancelled.into());
        }
        for input in &request.input {
            if let Input::User(text) = input
                && text.starts_with("Completed step record 3,")
            {
                assert!(text.contains("recall_address={"));
                assert!(text.contains("\"expected_call_id\":\"prior-read-x\""));
                let (_, content) = text.split_once('\n').unwrap();
                self.accepted.lock().unwrap().push(content.into());
            }
        }
        Ok(session::tests::handoff_response(
            request,
            "Saved the covered prefix of this exact read.",
        ))
    }
}

fn context() -> Context {
    let (events, _received) = std::sync::mpsc::sync_channel(64);
    Context {
        events,
        cancelled: Arc::new(AtomicBool::new(false)),
        stopped: Arc::new(AtomicBool::new(false)),
        guidance: Arc::new(session::queue::Pending::default()),
        next_effect: AtomicU64::new(1),
        effect_gate: None,
    }
}

#[test]
fn baseline_reference_checkpoint_resumes_with_exact_remaining_coverage() {
    let home = crate::state::tests::Fixture::new();
    let store = home.store().unwrap();
    let mut history = session::persistence::create_in(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history
        .begin("Continue the original large read".into())
        .unwrap();
    let output = "\u{0001}\u{0001}é€".repeat(140_000);
    assert_eq!(output.len(), 980_000);
    history.turns[0].steps.push(Step {
        response: Some(session::tool_tests::calls_response(vec![
            session::tool_tests::call("prior-read-x", "read_file", r#"{"path":"big.txt"}"#),
        ])),
        results: vec![Receipt {
            call_id: "prior-read-x".into(),
            output: output.clone(),
            summary: "Read big.txt".into(),
            image: None,
        }],
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    history.checkpoint().unwrap();
    let original = reference_at_position(&history, 0, 0, 3).unwrap().content;
    // These geometry values came from the pre-address baseline. A saved pending
    // byte offset is meaningful only while this serialized content stays exact.
    assert_eq!(original.len(), LEGACY_RECORD_BYTES);
    let digest = crate::tls::crypto::sha256::Sha256::digest(original.as_bytes());
    assert_eq!(
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        LEGACY_REFERENCE_SHA256
    );
    assert_eq!(
        original.find("\"receipt_output\":\"").unwrap() + "\"receipt_output\":\"".len(),
        LEGACY_RECEIPT_START
    );
    assert!(original.contains("é€") && original.contains("\\u0001"));

    let accepted = Arc::new(Mutex::new(Vec::new()));
    let mut first = Slices {
        accepted: accepted.clone(),
        cancel_at: Some(2),
        calls: 0,
    };
    assert_eq!(
        session::context::compact(
            &mut first,
            &mut history,
            &context(),
            Model::Luna,
            false,
            &mut session::Metrics::default(),
        ),
        Err(Failure::Cancelled)
    );
    let pending = history.projection.pending.as_ref().unwrap();
    assert_eq!(pending.record, 3);
    assert!(pending.offset > LEGACY_RECEIPT_START && pending.offset < original.len());
    assert!(original.is_char_boundary(pending.offset));
    assert_eq!(
        accepted.lock().unwrap().concat(),
        original[..pending.offset]
    );
    let pending_offset = pending.offset;
    drop(history);

    let mut saved = session::persistence::load(&store, &id, true).unwrap();
    let pending = saved.history.projection.pending.as_ref().unwrap();
    assert_eq!((pending.record, pending.offset), (3, pending_offset));
    assert_eq!(
        reference_at_position(&saved.history, 0, 0, 3)
            .unwrap()
            .content,
        original
    );
    let mut continuation = Slices {
        accepted: accepted.clone(),
        cancel_at: None,
        calls: 0,
    };
    session::context::compact(
        &mut continuation,
        &mut saved.history,
        &context(),
        Model::Luna,
        false,
        &mut session::Metrics::default(),
    )
    .unwrap();
    assert!(saved.history.projection.pending.is_none());
    assert_eq!(accepted.lock().unwrap().concat(), original);
    let canonical = saved
        .history
        .record
        .as_ref()
        .unwrap()
        .recorded_turn(0)
        .unwrap();
    assert_eq!(canonical.steps[0].results[0].output, output);
    drop(saved);
    let reopened = session::persistence::load(&store, &id, true).unwrap();
    assert!(reopened.history.projection.pending.is_none());
    assert_eq!(
        reopened
            .history
            .record
            .as_ref()
            .unwrap()
            .recorded_turn(0)
            .unwrap()
            .steps[0]
            .results[0]
            .output,
        output
    );
}
