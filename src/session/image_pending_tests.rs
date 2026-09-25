//! Visual evidence must remain in the projected request until a validated visual response.
//! These lifecycle tests share one synthetic provider and canonical fixture builders;
//! keeping them together makes failure, resume, model and compaction transitions auditable.
use super::*;
use crate::{
    image::{data_url, fixture_png, fixture_png_padded},
    providers::openai_account::{Input, Progress, Request, Response, Status, client},
    tls::Budget,
    workspace::Workspace,
    workspace_fixture,
};
use std::{
    ops::ControlFlow,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Instant,
};

pub(super) fn request_images(request: &Request) -> Vec<(String, String)> {
    request
        .input
        .iter()
        .filter_map(|item| match item {
            Input::ToolImage {
                call_id, image_url, ..
            } => Some((call_id.clone(), image_url.clone())),
            _ => None,
        })
        .collect()
}

#[derive(Default)]
struct InterruptedImageBackend {
    calls: usize,
    requests: Vec<(bool, Vec<(String, String)>)>,
}
impl worker::Backend for InterruptedImageBackend {
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
        let summary = request.instructions.starts_with("Summarize");
        let images = request_images(request);
        self.requests.push((summary, images.clone()));
        if summary {
            return Ok(tests::handoff_response(
                request,
                "The task is unfinished. Inspect the saved PNG before describing it.",
            ));
        }
        self.calls += 1;
        match self.calls {
            1 => Ok(tool_tests::calls_response(vec![tool_tests::call(
                "image-call",
                "view_image",
                r#"{"path":"screen.png"}"#,
            )])),
            2 => {
                assert_eq!(images.len(), 1);
                Err(client::Error::Status(503))
            }
            _ => Ok(tests::response("Continuation response", Status::Completed)),
        }
    }
}

pub(super) fn test_context() -> (worker::Context, mpsc::Receiver<Event>) {
    let (events, received) = mpsc::sync_channel(64);
    (
        worker::Context {
            events,
            cancelled: Arc::new(AtomicBool::new(false)),
            stopped: Arc::new(AtomicBool::new(false)),
            guidance: Arc::new(queue::Pending::default()),
            next_effect: AtomicU64::new(1),
            effect_gate: None,
        },
        received,
    )
}

fn failed_request_then_continue(resume: bool) {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let png = fixture_png_padded([13, 29, 47, 255], 500_000);
    std::fs::write(files.0.join("screen.png"), &png).unwrap();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    let id = history.record.as_ref().unwrap().id().to_owned();
    let (context, _events) = test_context();
    let mut backend = InterruptedImageBackend::default();
    history.begin("Inspect screen.png".into()).unwrap();
    history.checkpoint().unwrap();
    let mut metrics = Metrics::default();
    let failure = tool_loop::run(
        &mut backend,
        &mut history,
        &context,
        Model::Luna,
        Some(&workspace),
        Instant::now(),
        Instant::now,
        &mut metrics,
    )
    .unwrap_err();
    assert_eq!(metrics.tool_calls, 1);
    history.turns.last_mut().unwrap().end = Some(End::Failed(failure));
    history.turns.last_mut().unwrap().outcome = failure.to_string();
    history.checkpoint().unwrap();
    assert_eq!(backend.calls, 2);
    assert!(
        backend.requests[1].1 == vec![("image-call".into(), data_url(&png))],
        "first visual request omitted or changed the captured image"
    );
    std::fs::remove_file(files.0.join("screen.png")).unwrap();
    let mut history = if resume {
        drop(history);
        persistence::load(&store, &id, true).unwrap().history
    } else {
        history
    };
    history.image_capable = true;
    history.begin("Continue".into()).unwrap();
    history.checkpoint().unwrap();
    let mut continuation = Metrics::default();
    assert_eq!(
        tool_loop::run(
            &mut backend,
            &mut history,
            &context,
            Model::Luna,
            Some(&workspace),
            Instant::now(),
            Instant::now,
            &mut continuation,
        ),
        Ok(End::Complete)
    );
    assert_eq!(
        continuation.tool_calls, 0,
        "historical tools must not replay"
    );
    assert_eq!(
        backend.calls, 3,
        "a failed request must not retry automatically"
    );
    assert!(
        backend.requests.last().unwrap().1 == vec![("image-call".into(), data_url(&png))],
        "continuation omitted or changed the saved image"
    );
}

#[test]
fn astra_unobserved_pixels_survive_failed_request_resume_and_continuation() {
    failed_request_then_continue(true);
}

#[test]
fn unobserved_pixels_survive_failed_request_and_same_process_continuation() {
    failed_request_then_continue(false);
}

pub(super) enum Reply {
    Complete(&'static str),
    Incomplete,
    Cancelled,
}

pub(super) struct ScriptBackend {
    replies: std::collections::VecDeque<Reply>,
    requests: Vec<(bool, Vec<(String, String)>)>,
}
impl ScriptBackend {
    pub(super) fn new(replies: impl IntoIterator<Item = Reply>) -> Self {
        Self {
            replies: replies.into_iter().collect(),
            requests: Vec::new(),
        }
    }
}
impl worker::Backend for ScriptBackend {
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
        self.requests.push((
            request.instructions.starts_with("Summarize"),
            request_images(request),
        ));
        match self
            .replies
            .pop_front()
            .expect("unexpected provider request")
        {
            Reply::Complete(text) if request.instructions.starts_with("Summarize") => {
                Ok(tests::handoff_response(request, text))
            }
            Reply::Complete(text) => Ok(tests::response(text, Status::Completed)),
            Reply::Incomplete => Ok(tests::response(
                "Unfinished visual response",
                Status::Incomplete,
            )),
            Reply::Cancelled => Err(client::Error::Network(crate::tls::NetworkError::Cancelled)),
        }
    }
}

pub(super) fn add_text_turn(history: &mut history::History, text: &str) {
    history.begin("Earlier work".into()).unwrap();
    let turn = history.turns.last_mut().unwrap();
    turn.steps.push(history::Step {
        text: text.into(),
        response: Some(tests::response(text, Status::Completed)),
        accepted: true,
        ..Default::default()
    });
    turn.end = Some(End::Complete);
    turn.outcome = "Complete".into();
    history.checkpoint().unwrap();
}

pub(super) fn add_image_turn(history: &mut history::History, images: &[(&str, &str, &[u8])]) {
    let saved = history.images().unwrap();
    let mut calls = Vec::new();
    let mut receipts = Vec::new();
    for &(call_id, path, bytes) in images {
        let evidence = saved.capture(bytes, path).unwrap();
        calls.push(tool_tests::call(
            call_id,
            "view_image",
            &format!(r#"{{"path":"{path}"}}"#),
        ));
        receipts.push(history::Receipt {
            call_id: call_id.into(),
            output: "PNG captured".into(),
            summary: "PNG captured".into(),
            image: Some(evidence),
        });
    }
    history.begin("Inspect the screenshots".into()).unwrap();
    history.turns.last_mut().unwrap().steps.push(history::Step {
        response: Some(tool_tests::calls_response(calls)),
        results: receipts,
        accepted: true,
        ..Default::default()
    });
    history.checkpoint().unwrap();
}

pub(super) fn generate_once(
    history: &mut history::History,
    backend: &mut ScriptBackend,
    context: &worker::Context,
    model: Model,
) -> Result<(), Failure> {
    let request = history.request(model, true)?;
    let result = generation::generate(
        backend,
        &request,
        history.turns.last_mut().unwrap(),
        context,
        Instant::now(),
        &Instant::now,
        &mut Metrics::default(),
    );
    history.checkpoint()?;
    result
}

#[test]
fn cancellation_and_incomplete_response_keep_pixels_pending() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    let png = fixture_png([9, 8, 7, 255]);
    add_image_turn(&mut history, &[("image-call", "screen.png", &png)]);
    let (context, _) = test_context();
    let mut backend = ScriptBackend::new([
        Reply::Cancelled,
        Reply::Incomplete,
        Reply::Complete("Visual inspection complete"),
    ]);
    assert_eq!(
        generate_once(&mut history, &mut backend, &context, Model::Luna),
        Err(Failure::Cancelled)
    );
    assert!(history.pending_image());
    assert!(!history.turns[0].steps[1].validated_visual_input);
    assert_eq!(
        generate_once(&mut history, &mut backend, &context, Model::Luna),
        Ok(())
    );
    assert_eq!(
        history.turns[0].steps[2].response.as_ref().unwrap().status,
        Status::Incomplete
    );
    assert!(!history.turns[0].steps[2].validated_visual_input);
    assert!(history.pending_image());
    assert_eq!(
        generate_once(&mut history, &mut backend, &context, Model::Luna),
        Ok(())
    );
    assert!(history.turns[0].steps[3].validated_visual_input);
    assert!(!history.pending_image());
    assert_eq!(backend.requests.len(), 3);
    for (summary, images) in &backend.requests {
        assert!(!summary);
        assert!(
            images == &vec![("image-call".into(), data_url(&png))],
            "an unvalidated attempt must not remove or change the pixels"
        );
    }
}

#[test]
fn manual_compaction_only_summarizes_context_before_pending_images() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    add_text_turn(&mut history, &"earlier task detail ".repeat(2000));
    let png = fixture_png_padded([5, 6, 7, 255], 200_000);
    add_image_turn(&mut history, &[("image-call", "screen.png", &png)]);
    let (context, _) = test_context();
    let mut backend = ScriptBackend::new([Reply::Complete("Earlier task summarized.")]);
    assert_eq!(
        context::compact(
            &mut backend,
            &mut history,
            &context,
            Model::Luna,
            true,
            &mut Metrics::default(),
        ),
        Ok(())
    );
    assert_eq!(backend.requests, vec![(true, Vec::new())]);
    assert!(history.pending_image());
    let visual = history.request(Model::Luna, true).unwrap();
    assert!(
        request_images(&visual) == vec![("image-call".into(), data_url(&png))],
        "manual compaction removed pending pixels"
    );
    assert_eq!(
        context::compact(
            &mut backend,
            &mut history,
            &context,
            Model::Luna,
            true,
            &mut Metrics::default(),
        ),
        Ok(())
    );
    assert_eq!(
        backend.requests.len(),
        1,
        "pending step must not be summarized"
    );
}

#[test]
fn automatic_compaction_reduces_older_text_and_retains_pending_pixels() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    let older = "x".repeat(900_000);
    add_text_turn(&mut history, &older);
    add_text_turn(&mut history, &older);
    let png = fixture_png_padded([3, 5, 7, 255], crate::workspace::MAX_IMAGE_BYTES - 200);
    add_image_turn(&mut history, &[("image-call", "screen.png", &png)]);
    let (context, _) = test_context();
    let mut backend = ScriptBackend::new([Reply::Complete("Earlier text summarized.")]);
    assert_eq!(
        context::ensure(
            &mut backend,
            &mut history,
            &context,
            Model::Luna,
            true,
            &mut Metrics::default(),
        ),
        Ok(())
    );
    assert_eq!(backend.requests, vec![(true, Vec::new())]);
    assert!(history.pending_image());
    let request = history.request(Model::Luna, true).unwrap();
    assert!(
        request_images(&request) == vec![("image-call".into(), data_url(&png))],
        "automatic compaction removed pending pixels"
    );
    assert!(request.encode(history::MAX_REQUEST).is_ok());
}

#[test]
fn text_only_model_response_does_not_consume_pending_visual_evidence() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    let png = fixture_png([11, 22, 33, 255]);
    add_image_turn(&mut history, &[("image-call", "screen.png", &png)]);
    let id = history.record.as_ref().unwrap().id().to_owned();
    let text_model = Model::new("text-only", None).unwrap();
    history.set_model(text_model).unwrap();
    history.image_capable = false;
    let request = history.request(text_model, true).unwrap();
    assert!(request_images(&request).is_empty());
    assert!(request.input.iter().any(|item| {
        matches!(item, Input::ToolResult { call_id, output }
            if call_id == "image-call" && output.contains("pixels are not visible"))
    }));
    let (context, _) = test_context();
    let mut backend = ScriptBackend::new([Reply::Complete("I cannot inspect those pixels.")]);
    assert_eq!(
        generate_once(&mut history, &mut backend, &context, text_model),
        Ok(())
    );
    assert!(history.pending_image());
    assert!(!history.turns[0].steps[1].validated_visual_input);
    assert_eq!(backend.requests, vec![(false, Vec::new())]);
    history.set_model(Model::Luna).unwrap();
    drop(history);
    let mut resumed = persistence::load(&store, &id, true).unwrap().history;
    resumed.image_capable = true;
    assert!(resumed.pending_image());
    assert!(
        request_images(&resumed.request(Model::Luna, true).unwrap())
            == vec![("image-call".into(), data_url(&png))],
        "returning to the image-capable model must restore saved pixels"
    );
}

#[test]
fn multiple_pending_receipts_keep_call_ids_order_and_exact_saved_bytes() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    let first = fixture_png([1, 2, 3, 255]);
    let second = fixture_png([3, 2, 1, 255]);
    add_image_turn(
        &mut history,
        &[
            ("first-call", "first.png", &first),
            ("second-call", "second.png", &second),
        ],
    );
    let id = history.record.as_ref().unwrap().id().to_owned();
    drop(history);
    let mut resumed = persistence::load(&store, &id, true).unwrap().history;
    resumed.image_capable = true;
    assert_eq!(resumed.pending_image_step(), Some((0, 0)));
    let expected = vec![
        ("first-call".into(), data_url(&first)),
        ("second-call".into(), data_url(&second)),
    ];
    assert!(
        request_images(&resumed.request(Model::Luna, true).unwrap()) == expected,
        "resumed request lost an image or changed its tool association"
    );
    let (context, _) = test_context();
    let mut backend = ScriptBackend::new([Reply::Complete("Both screenshots inspected.")]);
    assert_eq!(
        generate_once(&mut resumed, &mut backend, &context, Model::Luna),
        Ok(())
    );
    assert!(!resumed.pending_image());
    assert!(backend.requests[0].1 == expected);
}

#[test]
fn validated_visual_response_allows_later_compaction() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    let session_id = history.record.as_ref().unwrap().id().to_owned();
    let png = fixture_png_padded([8, 6, 4, 255], 300_000);
    add_image_turn(&mut history, &[("image-call", "screen.png", &png)]);
    let image_id = history.turns[0].steps[0].results[0]
        .image
        .as_ref()
        .unwrap()
        .id
        .clone();
    let (context, _) = test_context();
    let mut backend = ScriptBackend::new([
        Reply::Complete("The screenshot shows the changed result."),
        Reply::Complete("Visual result verified; saved image can be revisited."),
    ]);
    assert_eq!(
        generate_once(&mut history, &mut backend, &context, Model::Luna),
        Ok(())
    );
    assert!(history.turns[0].steps[1].validated_visual_input);
    assert!(!history.pending_image());
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    history.checkpoint().unwrap();
    drop(history);
    let mut history = persistence::load(&store, &session_id, true)
        .unwrap()
        .history;
    history.image_capable = true;
    assert!(
        !history.pending_image(),
        "validated visual response must survive resume"
    );
    assert_eq!(
        context::compact(
            &mut backend,
            &mut history,
            &context,
            Model::Luna,
            true,
            &mut Metrics::default(),
        ),
        Ok(())
    );
    assert_eq!(backend.requests.len(), 2);
    assert!(!backend.requests[0].0);
    assert!(backend.requests[0].1 == vec![("image-call".into(), data_url(&png))]);
    assert_eq!(backend.requests[1], (true, Vec::new()));
    assert!(!history.pending_image());
    assert!(request_images(&history.request(Model::Luna, true).unwrap()).is_empty());
    assert_eq!(
        history.images().unwrap().load_id(&image_id).unwrap().id,
        image_id
    );
}

#[test]
fn visual_response_that_calls_another_image_keeps_the_new_receipt_pending() {
    let mut history = history::History::default();
    history.begin("Inspect two stages".into()).unwrap();
    history.turns[0].steps.push(history::Step {
        response: Some(tool_tests::calls_response(vec![tool_tests::call(
            "second-call",
            "view_image",
            r#"{"path":"second.png"}"#,
        )])),
        results: vec![history::Receipt {
            call_id: "second-call".into(),
            output: "PNG captured".into(),
            summary: "PNG captured".into(),
            image: Some(crate::image::Evidence {
                id: "a".repeat(64),
                path: "second.png".into(),
                width: 1,
                height: 1,
                bytes: 70,
            }),
        }],
        accepted: true,
        validated_visual_input: true,
        ..Default::default()
    });
    assert_eq!(history.pending_image_step(), Some((0, 0)));
}

#[test]
fn failed_response_checkpoint_does_not_durably_consume_pixels() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    let png = fixture_png([20, 30, 40, 255]);
    add_image_turn(&mut history, &[("image-call", "screen.png", &png)]);
    let id = history.record.as_ref().unwrap().id().to_owned();
    let (context, _) = test_context();
    let mut backend = ScriptBackend::new([Reply::Complete("A validated visual response.")]);
    let request = history.request(Model::Luna, true).unwrap();
    assert!(
        generation::generate(
            &mut backend,
            &request,
            history.turns.last_mut().unwrap(),
            &context,
            Instant::now(),
            &Instant::now,
            &mut Metrics::default(),
        )
        .is_ok()
    );
    assert!(history.turns[0].steps[1].validated_visual_input);
    history.fail_next_checkpoint.store(true, Ordering::Release);
    assert_eq!(history.checkpoint(), Err(Failure::Storage));
    drop(history);
    let mut resumed = persistence::load(&store, &id, true).unwrap().history;
    resumed.image_capable = true;
    assert_eq!(resumed.turns[0].steps.len(), 1);
    assert!(resumed.pending_image());
    assert!(
        request_images(&resumed.request(Model::Luna, true).unwrap())
            == vec![("image-call".into(), data_url(&png))],
        "uncommitted completion must not remove saved pixels"
    );
    assert_eq!(backend.requests.len(), 1);
}
