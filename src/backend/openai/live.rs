//! Opt-in diagnostics use the selected ChatGPT registration and synthetic text.
use super::*;
use crate::json::{self, Value};

fn item_shape(item: &Value) -> Value {
    let mut shape = Value::object([]);
    if let Value::Object(fields) = &mut shape {
        for key in [
            "type",
            "role",
            "channel",
            "phase",
            "name",
            "namespace",
            "status",
        ] {
            if let Some(value) = item.get(key) {
                fields.insert(key.into(), value.clone());
            }
        }
        if let Value::Object(item) = item {
            fields.insert(
                "keys".into(),
                Value::Array(item.keys().map(Value::string).collect()),
            );
        }
        if let Some(parts) = item.get("content").and_then(Value::as_array) {
            fields.insert(
                "content".into(),
                Value::Array(parts.iter().map(item_shape).collect()),
            );
        }
    }
    shape
}

#[test]
#[ignore = "uses the selected real ChatGPT account for a minimal synthetic diagnosis"]
fn inspect_live_model_and_stream() {
    let settings = crate::config::Store::discover().unwrap();
    let home = settings.path().parent().unwrap();
    let session = crate::auth::openai::Store::new(home.to_path_buf())
        .active()
        .unwrap()
        .expect("Sign in to OpenAI first");
    let api = Api::new(session).unwrap();
    let catalog = api.get("/models").unwrap();
    let row = catalog
        .get("models")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|row| row.get("slug").and_then(Value::as_str) == Some("gpt-6.1-sol"))
        .expect("gpt-6.1-sol unavailable");
    let scratch = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp");
    std::fs::create_dir_all(&scratch).unwrap();
    let mut safe_row = Value::object([]);
    if let Value::Object(fields) = &mut safe_row {
        for name in [
            "slug",
            "display_name",
            "visibility",
            "context_window",
            "max_context_window",
            "auto_compact_token_limit",
            "default_reasoning_level",
            "supported_reasoning_levels",
            "input_modalities",
        ] {
            if let Some(value) = row.get(name) {
                fields.insert(name.into(), value.clone());
            }
        }
    }
    std::fs::write(
        scratch.join("openai-live-catalog.json"),
        api.redactor().value(&safe_row).pretty(),
    )
    .unwrap();
    let body = Value::object([
        ("model", Value::string("gpt-6.1-sol")),
        (
            "input",
            Value::Array(vec![Value::object([
                ("role", Value::string("user")),
                (
                    "content",
                    Value::string(
                        "Reply with exactly these characters and nothing else: JECODE_LIVE_OK",
                    ),
                ),
            ])]),
        ),
        (
            "reasoning",
            Value::object([("effort", Value::string("low"))]),
        ),
        ("store", Value::Bool(false)),
        ("stream", Value::Bool(true)),
    ])
    .encode();
    let mut raw = Vec::new();
    let mut stream = stream::Stream::default();
    api.post_stream("/responses", &body, None, &mut |bytes| {
        raw.extend_from_slice(bytes);
        stream.feed(bytes, &mut |_| Ok(()))
    })
    .unwrap();
    let mut events = Vec::new();
    for line in std::str::from_utf8(&raw).unwrap().lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let Ok(event) = json::parse(data.trim()) else {
            continue;
        };
        let mut shape = item_shape(&event);
        if let Value::Object(fields) = &mut shape {
            if let Some(item) = event.get("item") {
                fields.insert("item".into(), item_shape(item));
            }
            if let Some(response) = event.get("response") {
                fields.insert("response".into(), item_shape(response));
                if let Some(output) = response.get("output").and_then(Value::as_array) {
                    fields.insert(
                        "output".into(),
                        Value::Array(output.iter().map(item_shape).collect()),
                    );
                }
            }
        }
        events.push(shape);
    }
    let shapes = Value::Array(events).pretty();
    std::fs::write(scratch.join("openai-live-events.json"), &shapes).unwrap();
    let result = stream.finish();
    match &result {
        Ok(completion) => eprintln!(
            "Live completion: text bytes {}, calls {}",
            completion.text.len(),
            completion.calls.len()
        ),
        Err(error) => eprintln!("Live completion: {}", error.message),
    }
    assert!(
        result.is_ok(),
        "Inspect .tmp/openai-live-events.json for sanitized event shapes"
    );
    assert_eq!(result.unwrap().text.trim(), "JECODE_LIVE_OK");
}

#[test]
#[ignore = "uses the selected real ChatGPT account with a synthetic local tool workload"]
fn live_tool_roundtrip_uses_context_and_selected_effort() {
    live_tool_workload(None, "JECODE_TOOL_OK");
}

#[test]
#[ignore = "uses the selected real ChatGPT account with an owned synthetic image and local tool"]
fn live_image_and_tool_roundtrip() {
    // Original 64x64 blue image generated with the Windows drawing API.
    const IMAGE: &str = "iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAYAAACqaXHeAAAAAXNSR0IArs4c6QAAAARnQU1BAACxjwv8YQUAAAAJcEhZcwAADsMAAA7DAcdvqGQAAACJSURBVHhe5cgxAQAADMOg+je9CYgEDh623dkSmoQmoUloEpqEJqFJaBKahCahSWgSmoQmoUloEpqEJqFJaBKahCahSWgSmoQmoUloEpqEJqFJaBKahCahSWgSmoQmoUloEpqEJqFJaBKahCahSWgSmoQmoUloEpqEJqFJaBKahCahSWgSmoQmYXlhqOHSNEsP9wAAAABJRU5ErkJggg==";
    let bytes = crate::attachments::base64::decode(IMAGE).unwrap();
    live_tool_workload(Some(&bytes), "BLUE");
}

fn live_tool_workload(image: Option<&[u8]>, expected: &str) {
    let settings = crate::config::Store::discover().unwrap();
    let session = crate::auth::openai::Store::new(settings.path().parent().unwrap().to_path_buf())
        .active()
        .unwrap()
        .expect("Sign in to OpenAI first");
    let api = Api::new(session).unwrap();
    let models = api.catalog().unwrap();
    let model = models
        .iter()
        .find(|model| model.id == "gpt-6.1-sol")
        .unwrap();
    assert!(model.efforts.contains(&Effort::High));
    assert!(model.efforts.contains(&Effort::Ultra));
    let mut backend = OpenAi::with_api(api, model.id.clone()).unwrap();
    backend.set_effort(Effort::High);
    let context = backend
        .known_limits()
        .expect("Model catalog has no context window")
        .context;
    assert!(context > 0);
    let project = crate::test_support::Directory::new();
    let home = crate::test_support::Directory::new();
    std::fs::write(
        project.path().join("probe.txt"),
        "Jecode live tool verification fixture.",
    )
    .unwrap();
    let mut agent = crate::agent::Agent::with_backend(
        Box::new(backend),
        crate::tools::Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let started = std::time::Instant::now();
    let prompt = if let Some(image) = image {
        let path = project.path().join("probe.png");
        std::fs::write(&path, image).unwrap();
        let attachments = agent.import_attachments(&[path]).unwrap();
        crate::attachments::Prompt::new("Use the read tool to read probe.txt. Then name the dominant color in the attached image in one English word, uppercase.".into(), attachments)
    } else {
        crate::attachments::Prompt::plain(
            "Use the read tool to read probe.txt. After reading it, reply with exactly JECODE_TOOL_OK and nothing else.",
        )
    };
    agent.run_turn(prompt, &mut |_| Ok(())).unwrap();
    let document = agent.archive().document();
    let messages = document.get("messages").and_then(Value::as_array).unwrap();
    assert!(
        messages
            .iter()
            .any(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
    );
    assert_eq!(
        messages
            .last()
            .unwrap()
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .trim(),
        expected
    );
    assert_eq!(agent.context_status().capacity, Some(context));
    eprintln!(
        "Live tool roundtrip completed in {:.1}s with known context and high effort",
        started.elapsed().as_secs_f64()
    );
}
