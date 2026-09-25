//! Aggregate image admission and explicit recovery through the real tool loop.
//! The fixtures keep request size, call association and exact pixel assertions together.
use super::image_pending_tests::{
    Reply, ScriptBackend, add_image_turn, add_text_turn, generate_once, request_images,
    test_context,
};
use super::*;
use crate::{
    image::{data_url, fixture_png, fixture_png_padded},
    providers::openai_account::{Input, Progress, Request, Response, Status, client},
    tls::Budget,
    workspace::Workspace,
    workspace_fixture,
};
use std::{ops::ControlFlow, time::Instant};

type ImageItems = Vec<(String, String)>;
type BatchRequest = (usize, ImageItems, Vec<(String, String)>);
type AdmissionRequest = (bool, usize, ImageItems, Vec<String>);

#[derive(Default)]
struct BatchImageBackend {
    calls: usize,
    requests: Vec<BatchRequest>,
}
impl worker::Backend for BatchImageBackend {
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
        self.requests.push((
            request.encode(history::MAX_REQUEST).unwrap().len(),
            request_images(request),
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
        ));
        if self.calls == 1 {
            Ok(tool_tests::calls_response(vec![
                tool_tests::call("first", "view_image", r#"{"path":"first.png"}"#),
                tool_tests::call("second", "view_image", r#"{"path":"second.png"}"#),
            ]))
        } else if self.calls == 2 {
            Ok(tool_tests::calls_response(vec![tool_tests::call(
                "smaller",
                "view_image",
                r#"{"path":"small.png"}"#,
            )]))
        } else {
            Ok(tests::response("Continue the task", Status::Completed))
        }
    }
}

#[test]
fn astra_oversized_image_batch_does_not_permanently_block_continuation() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let first = fixture_png_padded([1, 2, 3, 255], 3_200_000);
    let small = fixture_png([7, 8, 9, 255]);
    std::fs::write(files.0.join("first.png"), &first).unwrap();
    std::fs::write(
        files.0.join("second.png"),
        fixture_png_padded([4, 5, 6, 255], 3_200_000),
    )
    .unwrap();
    std::fs::write(files.0.join("small.png"), &small).unwrap();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    let (context, _events) = test_context();
    let mut backend = BatchImageBackend::default();
    history.begin("Inspect both screenshots".into()).unwrap();
    history.checkpoint().unwrap();
    let mut metrics = Metrics::default();
    assert_eq!(
        tool_loop::run(
            &mut backend,
            &mut history,
            &context,
            Model::Luna,
            Some(&workspace),
            Instant::now(),
            Instant::now,
            &mut metrics
        ),
        Ok(End::Complete)
    );
    assert_eq!(metrics.tool_calls, 3);
    assert_eq!(backend.calls, 3);
    assert!(
        backend.requests[1].1 == vec![("first".into(), data_url(&first))],
        "first request must contain exact accepted pixels"
    );
    assert_eq!(backend.requests[1].2.len(), 1);
    assert_eq!(backend.requests[1].2[0].0, "second");
    assert!(backend.requests[1].2[0].1.contains("8 MiB"));
    assert!(
        backend.requests[2].1
            == vec![
                ("first".into(), data_url(&first)),
                ("smaller".into(), data_url(&small))
            ],
        "next request must contain accepted and smaller pixels"
    );
    assert_eq!(backend.requests[2].2[0].0, "second");
    assert!(
        backend
            .requests
            .iter()
            .all(|(bytes, _, _)| *bytes <= history::MAX_REQUEST)
    );
    assert_eq!(
        history.turns[0].steps[0]
            .results
            .iter()
            .filter(|r| r.image.is_some())
            .count(),
        1
    );
    assert!(history.turns[0].steps[0].results[1].image.is_none());
    assert!(
        history.turns[0].steps[0].results[1]
            .output
            .contains("view_image was rejected")
    );
}

struct AdmissionBackend {
    replies: std::collections::VecDeque<Response>,
    requests: Vec<AdmissionRequest>,
}
impl worker::Backend for AdmissionBackend {
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
        self.requests.push((
            summary,
            request.encode(history::MAX_REQUEST).unwrap().len(),
            request_images(request),
            request
                .input
                .iter()
                .filter_map(|item| match item {
                    Input::ToolResult { call_id, .. } => Some(call_id.clone()),
                    _ => None,
                })
                .collect(),
        ));
        if summary {
            Ok(tests::response(
                "Earlier text work is complete; inspect the requested screenshots.",
                Status::Completed,
            ))
        } else {
            Ok(self
                .replies
                .pop_front()
                .expect("unexpected generation request"))
        }
    }
}

#[test]
fn saved_id_duplicate_views_obey_the_aggregate_budget() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    let png = fixture_png_padded([12, 34, 56, 255], 3_200_000);
    let saved = history
        .images()
        .unwrap()
        .capture(&png, "original.png")
        .unwrap();
    let argument = format!(r#"{{"path":null,"image_id":"{}"}}"#, saved.id);
    let mut backend = AdmissionBackend {
        replies: [
            tool_tests::calls_response(vec![
                tool_tests::call("saved-first", "view_image", &argument),
                tool_tests::call("saved-again", "view_image", &argument),
            ]),
            tests::response("One saved view was accepted.", Status::Completed),
        ]
        .into(),
        requests: Vec::new(),
    };
    let (context, _events) = test_context();
    history
        .begin("Inspect saved screenshot twice".into())
        .unwrap();
    history.checkpoint().unwrap();
    let mut metrics = Metrics::default();
    assert_eq!(
        tool_loop::run(
            &mut backend,
            &mut history,
            &context,
            Model::Luna,
            Some(&workspace),
            Instant::now(),
            Instant::now,
            &mut metrics
        ),
        Ok(End::Complete)
    );
    assert_eq!(metrics.tool_calls, 2);
    assert_eq!(backend.requests.len(), 2);
    assert!(
        backend.requests[1].2 == vec![("saved-first".into(), data_url(&png))],
        "saved ID bytes must be exact"
    );
    assert_eq!(backend.requests[1].3, vec!["saved-again"]);
    assert_eq!(
        history.turns[0].steps[0]
            .results
            .iter()
            .filter(|r| r.image.is_some())
            .count(),
        1
    );
    assert_eq!(history.images().unwrap().load(&saved).unwrap(), png);
}

#[test]
fn image_admission_compacts_eligible_text_before_rejecting_a_view() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let first = fixture_png_padded([2, 4, 6, 255], 3_040_000);
    let second = fixture_png_padded([3, 5, 7, 255], 3_040_000);
    std::fs::write(files.0.join("first.png"), &first).unwrap();
    std::fs::write(files.0.join("second.png"), &second).unwrap();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    add_text_turn(&mut history, &"Earlier completed text. ".repeat(14000));
    let mut backend = AdmissionBackend {
        replies: [
            tool_tests::calls_response(vec![
                tool_tests::call("first", "view_image", r#"{"path":"first.png"}"#),
                tool_tests::call("second", "view_image", r#"{"path":"second.png"}"#),
            ]),
            tests::response("Both screenshots inspected.", Status::Completed),
        ]
        .into(),
        requests: Vec::new(),
    };
    let (context, _events) = test_context();
    history.begin("Inspect both screenshots".into()).unwrap();
    history.checkpoint().unwrap();
    let mut metrics = Metrics::default();
    assert_eq!(
        tool_loop::run(
            &mut backend,
            &mut history,
            &context,
            Model::Luna,
            Some(&workspace),
            Instant::now(),
            Instant::now,
            &mut metrics
        ),
        Ok(End::Complete)
    );
    assert_eq!(metrics.tool_calls, 2);
    assert_eq!(backend.requests.iter().filter(|r| r.0).count(), 1);
    let final_request = backend.requests.last().unwrap();
    assert!(
        final_request.2
            == vec![
                ("first".into(), data_url(&first)),
                ("second".into(), data_url(&second))
            ],
        "compaction should admit both images; received {} images and {} text results in {} encoded bytes",
        final_request.2.len(),
        final_request.3.len(),
        final_request.1
    );
    assert!(final_request.3.is_empty());
    assert!(backend.requests.iter().all(|r| r.1 <= history::MAX_REQUEST));
}

#[test]
fn oversized_saved_pending_batch_can_be_explicitly_replaced_after_resume() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let first = fixture_png_padded([10, 20, 30, 255], 3_200_000);
    let second = fixture_png_padded([40, 50, 60, 255], 3_200_000);
    std::fs::write(files.0.join("first.png"), &first).unwrap();
    std::fs::write(files.0.join("second.png"), &second).unwrap();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    add_image_turn(
        &mut history,
        &[
            ("old-first", "first.png", &first),
            ("old-second", "second.png", &second),
        ],
    );
    history.turns[0].end = Some(End::Failed(Failure::ImageRequestLimit));
    history.turns[0].outcome = Failure::ImageRequestLimit.to_string();
    history.checkpoint().unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    let original_receipts: Vec<_> = history.turns[0].steps[0]
        .results
        .iter()
        .map(|r| (r.output.clone(), r.image.clone().unwrap()))
        .collect();
    drop(history);
    std::fs::write(files.0.join("first.png"), fixture_png([1, 2, 3, 255])).unwrap();
    std::fs::write(files.0.join("second.png"), fixture_png([4, 5, 6, 255])).unwrap();
    let mut resumed = persistence::load(&store, &id, true).unwrap().history;
    resumed.image_capable = true;
    assert!(resumed.pending_image());
    assert!(matches!(
        resumed
            .projected_request(Model::Luna, true)
            .unwrap()
            .encode(history::MAX_REQUEST),
        Err(crate::providers::openai_account::Error::Json(
            crate::json::Error::Limit
        ))
    ));
    let (context, _events) = test_context();
    let mut backend = AdmissionBackend {
        replies: [
            tool_tests::calls_response(vec![
                tool_tests::call("new-first", "view_image", r#"{"path":"first.png"}"#),
                tool_tests::call("new-second", "view_image", r#"{"path":"second.png"}"#),
            ]),
            tests::response("New smaller screenshots inspected.", Status::Completed),
        ]
        .into(),
        requests: Vec::new(),
    };
    assert_eq!(
        context::compact(
            &mut backend,
            &mut resumed,
            &context,
            Model::Luna,
            true,
            &mut Metrics::default()
        ),
        Err(Failure::ImageRequestLimit)
    );
    assert!(backend.requests.is_empty());
    assert_eq!(resumed.abandon_pending_images(), Ok(true));
    assert!(!resumed.pending_image());
    drop(resumed);
    let mut resumed = persistence::load(&store, &id, true).unwrap().history;
    resumed.image_capable = true;
    assert!(!resumed.pending_image());
    let old_request = resumed.request(Model::Luna, true).unwrap();
    assert!(request_images(&old_request).is_empty());
    for (index, call_id) in ["old-first", "old-second"].iter().enumerate() {
        let receipt = &resumed.turns[0].steps[0].results[index];
        assert_eq!(receipt.call_id, *call_id);
        assert_eq!(receipt.output, original_receipts[index].0);
        assert_eq!(receipt.image.as_ref(), Some(&original_receipts[index].1));
        let expected = if index == 0 { &first } else { &second };
        assert!(
            resumed
                .images()
                .unwrap()
                .load(&original_receipts[index].1)
                .unwrap()
                == *expected,
            "old evidence changed"
        );
        assert!(old_request.input.iter().any(|item| matches!(item, Input::ToolResult { call_id: id, output } if id == call_id && output.contains("without a validated visual inspection"))));
    }
    assert_eq!(
        context::compact(
            &mut backend,
            &mut resumed,
            &context,
            Model::Luna,
            true,
            &mut Metrics::default(),
        ),
        Ok(())
    );
    assert_eq!(backend.requests.len(), 1);
    assert!(backend.requests[0].0);
    assert!(backend.requests[0].2.is_empty());
    assert!(request_images(&resumed.request(Model::Luna, true).unwrap()).is_empty());
    resumed
        .begin("Inspect the newly saved smaller screenshots".into())
        .unwrap();
    resumed.checkpoint().unwrap();
    let mut metrics = Metrics::default();
    assert_eq!(
        tool_loop::run(
            &mut backend,
            &mut resumed,
            &context,
            Model::Luna,
            Some(&workspace),
            Instant::now(),
            Instant::now,
            &mut metrics
        ),
        Ok(End::Complete)
    );
    assert_eq!(metrics.tool_calls, 2, "historical views must not replay");
    assert_eq!(backend.requests.len(), 3);
    assert!(backend.requests[1].2.is_empty());
    assert!(
        backend.requests[2].2
            == vec![
                ("new-first".into(), data_url(&fixture_png([1, 2, 3, 255]))),
                ("new-second".into(), data_url(&fixture_png([4, 5, 6, 255]))),
            ],
        "replacement views were not delivered exactly"
    );
    assert!(backend.requests.iter().all(|r| r.1 <= history::MAX_REQUEST));
}

#[test]
fn failed_discard_checkpoint_keeps_visual_evidence_pending() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    let png = fixture_png([9, 7, 5, 255]);
    add_image_turn(&mut history, &[("pending", "screen.png", &png)]);
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.fail_next_checkpoint.store(true, Ordering::Release);
    assert_eq!(history.abandon_pending_images(), Err(Failure::Storage));
    assert!(history.pending_image());
    assert!(history.projection.abandoned_visual.is_empty());
    drop(history);
    let mut resumed = persistence::load(&store, &id, true).unwrap().history;
    resumed.image_capable = true;
    assert!(resumed.pending_image());
    assert!(
        request_images(&resumed.request(Model::Luna, true).unwrap())
            == vec![("pending".into(), data_url(&png))],
        "failed checkpoint consumed the pending pixels"
    );
}

#[test]
fn discard_marks_only_unobserved_images_after_a_validated_visual_response() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    let observed = fixture_png([1, 2, 3, 255]);
    let pending = fixture_png([4, 5, 6, 255]);
    add_image_turn(&mut history, &[("observed", "old.png", &observed)]);
    let (context, _events) = test_context();
    let mut backend = ScriptBackend::new([Reply::Complete("The old image was inspected.")]);
    assert_eq!(
        generate_once(&mut history, &mut backend, &context, Model::Luna),
        Ok(())
    );
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    history.checkpoint().unwrap();
    add_image_turn(&mut history, &[("pending", "new.png", &pending)]);
    let id = history.record.as_ref().unwrap().id().to_owned();
    assert_eq!(history.pending_image_step(), Some((1, 0)));
    assert_eq!(history.abandon_pending_images(), Ok(true));
    drop(history);
    let mut resumed = persistence::load(&store, &id, true).unwrap().history;
    resumed.image_capable = true;
    assert!(!resumed.pending_image());
    let request = resumed.request(Model::Luna, true).unwrap();
    assert!(
        request_images(&request) == vec![("observed".into(), data_url(&observed))],
        "previously inspected pixels were incorrectly discarded"
    );
    assert!(request.input.iter().any(|item| matches!(item, Input::ToolResult { call_id, output } if call_id == "pending" && output.contains("without a validated visual inspection"))));
}

#[test]
fn local_discard_command_recovers_a_blocked_saved_session_without_provider_work() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    let first = fixture_png_padded([1, 3, 5, 255], 3_200_000);
    let second = fixture_png_padded([2, 4, 6, 255], 3_200_000);
    add_image_turn(
        &mut history,
        &[("old-a", "a.png", &first), ("old-b", "b.png", &second)],
    );
    history.turns[0].end = Some(End::Failed(Failure::ImageRequestLimit));
    history.turns[0].outcome = Failure::ImageRequestLimit.to_string();
    history.checkpoint().unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    let backend = AdmissionBackend {
        replies: [].into(),
        requests: Vec::new(),
    };
    let mut session =
        Session::with_history(Model::Luna, backend, Some(workspace), history).unwrap();
    assert!(matches!(tests::next(&mut session), Event::Restored { .. }));
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.discard_pending_images());
    assert!(
        matches!(tests::next(&mut session), Event::ContextReport(text) if text.contains("without inspection"))
    );
    assert!(matches!(
        tests::next(&mut session),
        Event::Finished(End::Complete, _)
    ));
    drop(session);
    let mut resumed = persistence::load(&store, &id, true).unwrap().history;
    resumed.image_capable = true;
    assert!(!resumed.pending_image());
    assert!(request_images(&resumed.request(Model::Luna, true).unwrap()).is_empty());
}
