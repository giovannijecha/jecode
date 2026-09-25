//! One fixture backend exercises the image contract across tool execution,
//! request encoding, model changes, persistence and compaction. Keeping these
//! lifecycle tests together avoids duplicate session and fake-home scaffolding.
use super::*;
use crate::{
    image::{Images, fixture_png, fixture_png_padded},
    json::{self, Value},
    providers::openai_account::{Progress, Request, Response, Status, client},
    tls::Budget,
    workspace::{Access, Workspace},
    workspace_fixture,
};
use std::{
    ops::ControlFlow,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Instant,
};

struct ImagesBackend {
    requests: Arc<Mutex<Vec<Value>>>,
    calls: usize,
    catalog: Option<crate::providers::openai_account::catalog::Catalog>,
}
impl worker::Backend for ImagesBackend {
    fn catalog(
        &mut self,
        _: &Budget<'_>,
    ) -> Result<Option<crate::providers::openai_account::catalog::Catalog>, client::Error> {
        Ok(self.catalog.clone())
    }
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
        let body = request.encode(history::MAX_REQUEST)?;
        self.requests.lock().unwrap().push(
            json::parse(
                &body,
                json::Limits {
                    bytes: history::MAX_REQUEST,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        if request.instructions.starts_with("Summarize") {
            let id = self
                .requests
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .get("input")
                .and_then(Value::array)
                .unwrap()
                .iter()
                .map(|item| json::encode(item, history::MAX_REQUEST).unwrap())
                .find_map(|item| {
                    item.find("image_id ")
                        .map(|start| item[start + 9..].chars().take(64).collect::<String>())
                })
                .unwrap_or_default();
            return Ok(tests::handoff_response(
                request,
                &format!(
                    "The viewed screenshot showed a red pixel. Stored image_id {id} remains available for re-view."
                ),
            ));
        }
        self.calls += 1;
        if self.calls == 1 {
            Ok(tool_tests::calls_response(vec![tool_tests::call(
                "image-call",
                "view_image",
                r#"{"path":"screen.png","image_id":null}"#,
            )]))
        } else {
            Ok(tests::response(
                "The screenshot contains a red pixel.",
                Status::Completed,
            ))
        }
    }
}

fn test_context() -> (worker::Context, mpsc::Receiver<Event>) {
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
fn image_item(request: &Value) -> Option<&Value> {
    request
        .get("input")?
        .array()?
        .iter()
        .find(|item| {
            item.get("call_id").and_then(Value::text) == Some("image-call")
                && item.get("output").and_then(Value::array).is_some()
        })?
        .get("output")?
        .array()?
        .iter()
        .find(|item| item.get("type").and_then(Value::text) == Some("input_image"))
}

fn ready(session: &mut Session) {
    loop {
        if matches!(tests::next(session), Event::Ready) {
            return;
        }
    }
}
fn finished(session: &mut Session) -> End {
    loop {
        if let Event::Finished(end, _) = tests::next(session) {
            return end;
        }
    }
}

#[test]
fn catalog_capability_controls_tool_exposure_and_retained_image_projection() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    std::fs::write(files.0.join("screen.png"), fixture_png([255, 0, 0, 255])).unwrap();
    let workspace = Workspace::open(&files.0).unwrap();
    let history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    let catalog = crate::providers::openai_account::catalog::Catalog::parse(br#"{"models":[{"slug":"gpt-5.6-luna","visibility":"list","input_modalities":["text","image"]},{"slug":"text-only","visibility":"list","input_modalities":["text"]}]}"#).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history_shell(
        Model::Luna,
        ImagesBackend {
            requests: requests.clone(),
            calls: 0,
            catalog: Some(catalog),
        },
        Some(workspace),
        history,
        crate::command::Shell::default(),
    )
    .unwrap();
    ready(&mut session);
    assert!(session.submit("Inspect screen.png"));
    assert_eq!(finished(&mut session), End::Complete);
    let captured = requests.lock().unwrap();
    let tools = captured[0].get("tools").and_then(Value::array).unwrap();
    let view = tools
        .iter()
        .find(|tool| tool.get("name").and_then(Value::text) == Some("view_image"))
        .unwrap();
    assert_eq!(view.get("strict"), Some(&Value::Bool(true)));
    let schema = view.get("parameters").unwrap();
    assert_eq!(
        schema.get("additionalProperties"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        schema.get("required").and_then(Value::array),
        Some(
            &[
                Value::String("path".into()),
                Value::String("image_id".into())
            ][..]
        )
    );
    for selector in ["path", "image_id"] {
        assert_eq!(
            schema
                .get("properties")
                .and_then(|properties| properties.get(selector))
                .and_then(|property| property.get("type"))
                .and_then(Value::array),
            Some(&[Value::String("string".into()), Value::String("null".into())][..])
        );
    }
    assert!(
        tools
            .iter()
            .filter(|tool| tool.get("name").and_then(Value::text) != Some("view_image"))
            .all(|tool| tool.get("strict").is_none())
    );
    assert!(image_item(&captured[1]).is_some());
    drop(captured);
    let text_model = Model::new("text-only", None).unwrap();
    assert!(session.set_model(text_model));
    assert!(matches!(tests::next(&mut session), Event::ModelChanged(model) if model == text_model));
    assert!(session.submit("Continue with saved evidence"));
    assert_eq!(finished(&mut session), End::Complete);
    let captured = requests.lock().unwrap();
    assert!(
        !captured[2]
            .get("tools")
            .and_then(Value::array)
            .unwrap()
            .iter()
            .any(|tool| tool.get("name").and_then(Value::text) == Some("view_image"))
    );
    assert!(image_item(&captured[2]).is_none());
    let text_request = json::encode(&captured[2], history::MAX_REQUEST).unwrap();
    assert!(text_request.contains("pixels are not visible"));
    drop(captured);
    assert!(session.set_model(Model::Luna));
    assert!(matches!(
        tests::next(&mut session),
        Event::ModelChanged(Model::Luna)
    ));
    assert!(session.submit("Reassess the saved screenshot"));
    assert_eq!(finished(&mut session), End::Complete);
    assert!(image_item(&requests.lock().unwrap()[3]).is_some());
}

#[test]
fn missing_image_metadata_and_conversation_only_stay_tool_free() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    let unknown = crate::providers::openai_account::catalog::Catalog::parse(
        br#"{"models":[{"slug":"gpt-5.6-luna","visibility":"list"}]}"#,
    )
    .unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history_shell(
        Model::Luna,
        ImagesBackend {
            requests: requests.clone(),
            calls: 0,
            catalog: Some(unknown),
        },
        Some(workspace),
        history,
        crate::command::Shell::default(),
    )
    .unwrap();
    ready(&mut session);
    assert!(session.submit("Inspect screenshot"));
    assert_eq!(finished(&mut session), End::Complete);
    assert!(
        !requests.lock().unwrap()[0]
            .get("tools")
            .and_then(Value::array)
            .unwrap()
            .iter()
            .any(|tool| tool.get("name").and_then(Value::text) == Some("view_image"))
    );
    drop(session);
    let no_workspace = persistence::create_in(&store, Model::Luna, Some(&files.0), None).unwrap();
    let supported = crate::providers::openai_account::catalog::Catalog::parse(br#"{"models":[{"slug":"gpt-5.6-luna","visibility":"list","input_modalities":["text","image"]}]}"#).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut conversation = Session::with_history_shell(
        Model::Luna,
        ImagesBackend {
            requests: requests.clone(),
            calls: 0,
            catalog: Some(supported),
        },
        None,
        no_workspace,
        crate::command::Shell::default(),
    )
    .unwrap();
    ready(&mut conversation);
    assert!(conversation.submit("Talk through screenshot plans"));
    assert_eq!(
        finished(&mut conversation),
        End::Failed(Failure::UnexpectedTools)
    );
    assert!(
        requests.lock().unwrap()[0]
            .get("tools")
            .and_then(Value::array)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn image_view_is_multimodal_and_survives_changed_source_resume_and_model_change() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let first = fixture_png_padded([255, 0, 0, 255], 90_000);
    std::fs::write(files.0.join("screen.png"), &first).unwrap();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    history.projection.limit_bytes = 65_536;
    let id = history.record.as_ref().unwrap().id().to_owned();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = ImagesBackend {
        requests: requests.clone(),
        calls: 0,
        catalog: None,
    };
    let (context, events) = test_context();
    history.begin("Inspect the screenshot".into()).unwrap();
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
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    history.checkpoint().unwrap();
    assert_eq!((metrics.requests, metrics.tool_calls), (2, 1));
    let evidence = history.turns[0].steps[0].results[0].image.clone().unwrap();
    assert_eq!(
        (evidence.width, evidence.height, evidence.bytes),
        (1, 1, first.len())
    );
    let captured = requests.lock().unwrap();
    assert_eq!(
        captured.len(),
        2,
        "a pending view must not be compacted before the model sees it"
    );
    assert!(
        captured[0]
            .get("tools")
            .and_then(Value::array)
            .unwrap()
            .iter()
            .any(|tool| tool.get("name").and_then(Value::text) == Some("view_image"))
    );
    let item = image_item(&captured[1]).unwrap();
    assert_eq!(item.get("detail").and_then(Value::text), Some("high"));
    assert_eq!(
        item.get("image_url").and_then(Value::text),
        Some(crate::image::data_url(&first).as_str())
    );
    assert_eq!(
        captured[1]
            .get("input")
            .and_then(Value::array)
            .unwrap()
            .iter()
            .filter(|item| item.get("call_id").and_then(Value::text) == Some("image-call"))
            .count(),
        2
    );
    assert!(
        !history.turns[0].steps[0].results[0]
            .output
            .contains("base64")
    );
    assert!(
        !history
            .transcript()
            .iter()
            .any(|item| item.text.contains("base64"))
    );
    for event in events.try_iter() {
        if let Event::ToolFinished { summary, .. } = event {
            assert!(summary.contains("PNG 1x1"));
            assert!(!summary.contains("base64"));
        }
    }
    drop(captured);
    std::fs::write(files.0.join("screen.png"), fixture_png([0, 0, 255, 255])).unwrap();
    std::fs::remove_file(files.0.join("screen.png")).unwrap();
    drop(history);
    let mut saved = persistence::load(&store, &id, true).unwrap().history;
    saved.image_capable = true;
    let resumed = json::parse(
        &saved
            .request(Model::Luna, true)
            .unwrap()
            .encode(history::MAX_REQUEST)
            .unwrap(),
        json::Limits {
            bytes: history::MAX_REQUEST,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        image_item(&resumed)
            .unwrap()
            .get("image_url")
            .and_then(Value::text),
        Some(crate::image::data_url(&first).as_str())
    );
    saved.image_capable = false;
    let text_only = saved
        .request(Model::Luna, true)
        .unwrap()
        .encode(history::MAX_REQUEST)
        .unwrap();
    assert!(!text_only.contains("input_image"));
    assert!(text_only.contains("pixels are not visible"));
    assert_eq!(
        saved.turns[0].steps[0].results[0]
            .image
            .as_ref()
            .unwrap()
            .id,
        evidence.id
    );
    saved.image_capable = true;
    let images = saved.images().unwrap();
    assert_eq!(images.load_id(&evidence.id).unwrap().id, evidence.id);
    let file = store
        .directory("images")
        .unwrap()
        .directory(&id)
        .unwrap()
        .root()
        .join(format!("{}.png", evidence.id));
    std::fs::write(&file, b"corrupt").unwrap();
    assert!(matches!(
        saved.request(Model::Luna, true),
        Err(Failure::ImageEvidence)
    ));
    std::fs::write(&file, &first).unwrap();
    let compact_requests = Arc::new(Mutex::new(Vec::new()));
    let mut compact_backend = ImagesBackend {
        requests: compact_requests.clone(),
        calls: 2,
        catalog: None,
    };
    let (compact_context, _) = test_context();
    assert_eq!(
        context::compact(
            &mut compact_backend,
            &mut saved,
            &compact_context,
            Model::Luna,
            true,
            &mut Metrics::default()
        ),
        Ok(())
    );
    let summary_request =
        json::encode(&compact_requests.lock().unwrap()[0], history::MAX_REQUEST).unwrap();
    assert!(summary_request.contains(&evidence.id));
    assert!(!summary_request.contains("input_image"));
    assert!(!summary_request.contains("base64"));
    assert!(saved.projection.summary.contains("red pixel"));
    assert!(saved.projection.summary.contains(&evidence.id));
    assert!(
        !saved
            .request(Model::Luna, true)
            .unwrap()
            .encode(history::MAX_REQUEST)
            .unwrap()
            .contains("input_image")
    );
    std::fs::write(&file, b"corrupt").unwrap();
    // Compaction leaves canonical evidence intact and does not need pixels to
    // continue; explicitly revisiting the image detects tampering.
    assert!(images.load(&evidence).is_err());
}

#[test]
fn image_tool_reports_format_path_policy_and_cancelled_capture() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let outside = workspace_fixture::Fixture::new();
    std::fs::write(files.0.join("valid.png"), fixture_png([1, 2, 3, 255])).unwrap();
    std::fs::write(files.0.join("bad.png"), b"not an image").unwrap();
    std::fs::write(files.0.join("photo.jpg"), [0xff, 0xd8, 0xff, 0xd9]).unwrap();
    std::fs::write(outside.0.join("outer.png"), fixture_png([4, 5, 6, 255])).unwrap();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    let (context, _events) = test_context();
    let run = |path: &str, access: &Workspace| {
        image_tool::execute(&history, access, Some(path), None, &context).unwrap()
    };
    assert!(
        run("missing.png", &workspace)
            .0
            .summary
            .contains("does not exist")
    );
    assert!(run("bad.png", &workspace).0.summary.contains("invalid PNG"));
    assert!(
        run("photo.jpg", &workspace)
            .0
            .summary
            .contains("unsupported image format")
    );
    assert!(run("../outer.png", &workspace).0.failed);
    let absolute = outside.0.join("outer.png").to_string_lossy().into_owned();
    assert!(run(&absolute, &workspace).0.failed);
    assert!(
        !run(
            &absolute,
            &Workspace::open(&files.0)
                .unwrap()
                .with_access(Access::Local)
        )
        .0
        .failed
    );
    let (view, saved) = run("valid.png", &workspace);
    assert!(!view.failed);
    let saved = saved.unwrap();
    let revisited =
        image_tool::execute(&history, &workspace, None, Some(&saved.id), &context).unwrap();
    assert_eq!(revisited.1.unwrap().id, saved.id);
    context.cancelled.store(true, Ordering::Release);
    assert!(run("valid.png", &workspace).0.failed);
    assert!(
        Images::in_store(&store, history.record.as_ref().unwrap().id())
            .unwrap()
            .load_id("bad-id")
            .is_err()
    );
}

#[test]
fn oversized_pending_visual_request_fails_before_any_provider_send() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    let png = fixture_png_padded([1, 2, 3, 255], crate::workspace::MAX_IMAGE_BYTES - 200);
    let other = fixture_png_padded([4, 5, 6, 255], crate::workspace::MAX_IMAGE_BYTES - 200);
    let first = history
        .images()
        .unwrap()
        .capture(&png, "screen-a.png")
        .unwrap();
    let second = history
        .images()
        .unwrap()
        .capture(&other, "screen-b.png")
        .unwrap();
    history.begin("Inspect the screenshot".into()).unwrap();
    history.turns[0].steps.push(history::Step {
        response: Some(tool_tests::calls_response(vec![
            tool_tests::call("image-a", "view_image", r#"{"path":"screen-a.png"}"#),
            tool_tests::call("image-b", "view_image", r#"{"path":"screen-b.png"}"#),
        ])),
        results: vec![
            history::Receipt {
                call_id: "image-a".into(),
                output: "PNG captured".into(),
                summary: "PNG captured".into(),
                image: Some(first),
            },
            history::Receipt {
                call_id: "image-b".into(),
                output: "PNG captured".into(),
                summary: "PNG captured".into(),
                image: Some(second),
            },
        ],
        accepted: true,
        ..Default::default()
    });
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = ImagesBackend {
        requests: requests.clone(),
        calls: 2,
        catalog: None,
    };
    let (context, _) = test_context();
    assert_eq!(
        context::ensure(
            &mut backend,
            &mut history,
            &context,
            Model::Luna,
            true,
            &mut Metrics::default(),
        ),
        Err(Failure::ImageRequestLimit)
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
        Err(Failure::ImageRequestLimit)
    );
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn failed_receipt_checkpoint_does_not_commit_a_successful_view() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let files = workspace_fixture::Fixture::new();
    std::fs::write(files.0.join("screen.png"), fixture_png([255, 0, 0, 255])).unwrap();
    let workspace = Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    history.image_capable = true;
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("Inspect".into()).unwrap();
    history.checkpoint().unwrap();
    let count = history.checkpoint_calls.load(Ordering::Acquire);
    history
        .fail_checkpoint_from
        .store(count + 2, Ordering::Release);
    let (context, _events) = test_context();
    let mut backend = ImagesBackend {
        requests: Arc::new(Mutex::new(Vec::new())),
        calls: 0,
        catalog: None,
    };
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
        Err(Failure::Storage)
    );
    drop(history);
    let saved = persistence::load(&store, &id, true).unwrap();
    assert!(saved.history.turns[0].steps[0].results[0].image.is_none());
    assert_eq!(
        saved.history.turns[0].steps[0].results[0].summary,
        "Not executed"
    );
}
