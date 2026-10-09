use super::*;
use crate::{
    attachments::{MARKER, Prompt},
    json,
    sessions::Store,
    test_support::{Directory, HttpFixture, completion, tool_call},
};

const PDF: &[u8] = b"%PDF-1.4\n1 0 obj << /Type /Page >> endobj\n%%EOF\n";

fn annotated(text: &str) -> Value {
    let annotation = r#""annotations":[{"type":"file","file":{"hash":"fixture-hash","name":"report.pdf","content":[{"type":"text","text":"parsed once"}]}}],"#;
    json::parse(&completion(text, vec![]).encode().replacen(
        "\"role\":",
        &format!("{annotation}\"role\":"),
        1,
    ))
    .unwrap()
}

fn annotated_image(text: &str, bytes: &[u8]) -> Value {
    let image = Value::object([
        ("type", Value::string("image_url")),
        (
            "image_url",
            Value::object([(
                "url",
                Value::string(format!(
                    "data:image/png;base64,{}",
                    crate::attachments::base64::encode(bytes)
                )),
            )]),
        ),
    ]);
    let annotation = Value::object([
        ("type", Value::string("file")),
        (
            "file",
            Value::object([
                ("hash", Value::string("fixture-hash")),
                ("name", Value::string("report.pdf")),
                (
                    "content",
                    Value::Array(vec![
                        Value::object([
                            ("type", Value::string("text")),
                            ("text", Value::string("parsed once")),
                        ]),
                        image,
                    ]),
                ),
            ]),
        ),
    ]);
    let annotations = Value::Array(vec![annotation]);
    json::parse(&completion(text, vec![]).encode().replacen(
        "\"role\":",
        &format!("\"annotations\":{},\"role\":", annotations.encode()),
        1,
    ))
    .unwrap()
}

fn parts(message: &Value) -> Vec<&str> {
    message
        .get("content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("type").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn attachments_are_sent_as_parts_and_pdf_annotations_are_replayed() {
    let directory = Directory::new();
    let home = Directory::new();
    let tools = Tools::new(directory.path()).unwrap();
    // The same bucket the agent will open, so fixture responses can name ids.
    let pool = Store::new(home.path().to_path_buf(), tools.root())
        .unwrap()
        .attachments();
    let pdf = pool.import_bytes("report.pdf", PDF).unwrap();
    let image = pool
        .import_bytes("shot.png", &crate::attachments::tests::png(4, 3))
        .unwrap();
    let fixture = HttpFixture::new(vec![
        (200, annotated("The report says hello.")),
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "look",
                    "read",
                    Value::object([("path", Value::string(image.reference()))]),
                )],
            ),
        ),
        (200, completion("Seen again.", vec![])),
    ]);
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    client.fixture_inputs(&["text", "image", "file"]);
    let mut agent = Agent::new(client, tools);
    agent.enable_sessions(home.path()).unwrap();
    agent
        .run_turn(
            Prompt::new(format!("Summarize {MARKER}"), vec![pdf.clone()]),
            &mut |_| Ok(()),
        )
        .unwrap();
    agent
        .run_turn("Look at the screenshot", &mut |_| Ok(()))
        .unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    let first = requests[0]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(parts(first.last().unwrap()), ["text", "file"]);
    assert_eq!(
        requests[0]
            .body
            .get("plugins")
            .map(Value::encode)
            .as_deref(),
        Some(r#"[{"id":"file-parser","pdf":{"engine":"native"}}]"#)
    );
    // The next request replays the parsed annotations with the same file.
    let second = requests[1]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap();
    let answer = second
        .iter()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
        .unwrap();
    assert!(
        answer
            .get("annotations")
            .unwrap()
            .encode()
            .contains("fixture-hash")
    );
    // A read of an image reference shows the image after the tool result.
    let third = requests[2]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap();
    let roles = third
        .iter()
        .rev()
        .take(2)
        .map(|message| message.get("role").and_then(Value::as_str).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(roles, ["user", "tool"]);
    assert_eq!(parts(third.last().unwrap()), ["text", "image_url"]);
    // History keeps references and annotations but never the bytes.
    let saved = agent.messages.lock().unwrap().clone();
    let encoded = Value::Array(saved).encode();
    assert!(encoded.contains(&pdf.id));
    assert!(encoded.contains("fixture-hash"));
    assert!(!encoded.contains("base64,"));
}

#[test]
fn pdf_annotation_images_stay_in_pool_across_save_resume_history_and_replay() {
    let directory = Directory::new();
    let home = Directory::new();
    let png = crate::attachments::tests::png(4, 3);
    let first = HttpFixture::new(vec![(200, annotated_image("Parsed report.", &png))]);
    let mut client = OpenRouter::fixture(first.endpoint.clone());
    client.fixture_inputs(&["text", "file"]);
    let mut agent = Agent::new(client, Tools::new(directory.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    let pdf = agent
        .sessions()
        .unwrap()
        .store()
        .attachments()
        .import_bytes("report.pdf", PDF)
        .unwrap();
    agent
        .run_turn(Prompt::new(MARKER.to_string(), vec![pdf]), &mut |_| Ok(()))
        .unwrap();
    let id = agent.sessions().unwrap().id();
    let saved = agent.messages.lock().unwrap().clone();
    let encoded = Value::Array(saved.clone()).encode();
    assert!(!encoded.contains("base64,"));
    assert!(encoded.contains("fixture-hash"));
    let refs = crate::attachments::annotations::references(&saved[2]);
    assert_eq!(refs.len(), 1);
    let asset = refs.iter().next().unwrap();
    let pool = agent.sessions().unwrap().store().attachments();
    assert_eq!(std::fs::read(pool.load(asset).unwrap().path).unwrap(), png);
    let history = agent.execute_tool(&ToolCall {
        id: "history-read".into(),
        name: "read".into(),
        arguments: Value::object([("path", Value::string("history:2"))]).encode(),
    });
    let history = history.get("content").and_then(Value::as_str).unwrap();
    assert!(history.contains(&format!("attachment:{asset}")));
    assert!(!history.contains("base64,"));
    let export = agent.archive().save().unwrap();
    let export_text = std::fs::read_to_string(&export).unwrap();
    assert!(!export_text.contains("base64,"));
    let export_document = json::parse(&export_text).unwrap();
    let listed = export_document
        .get("attachments")
        .and_then(Value::as_array)
        .unwrap();
    let parsed_page = listed
        .iter()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(asset))
        .unwrap();
    let path = parsed_page.get("path").and_then(Value::as_str).unwrap();
    assert_eq!(
        std::fs::read(export.parent().unwrap().join(path)).unwrap(),
        png
    );
    first.finish();
    drop(agent);

    let document = Store::new(home.path().to_path_buf(), directory.path())
        .unwrap()
        .fixture_load(&id)
        .unwrap();
    assert!(!Value::Array(document.messages).encode().contains("base64,"));
    let second = HttpFixture::new(vec![(200, completion("Continued.", vec![]))]);
    let mut client = OpenRouter::fixture(second.endpoint.clone());
    client.fixture_inputs(&["text", "file"]);
    let mut resumed = Agent::new(client, Tools::new(directory.path()).unwrap());
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    resumed.run_turn("Continue", &mut |_| Ok(())).unwrap();
    let requests = second.finish();
    let messages = requests[0]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap();
    let answer = messages
        .iter()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
        .unwrap();
    let annotations = answer.get("annotations").unwrap().encode();
    assert!(annotations.contains("fixture-hash"));
    assert!(annotations.contains(&format!(
        "data:image/png;base64,{}",
        crate::attachments::base64::encode(&png)
    )));
}

#[test]
fn annotation_storage_failure_keeps_the_answer_without_inline_data() {
    let directory = Directory::new();
    let png = crate::attachments::tests::png(4, 3);
    let fixture = HttpFixture::new(vec![(200, annotated_image("Answer remains.", &png))]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let mut shown = String::new();
    let error = agent
        .run_turn("Read a PDF", &mut |event| {
            if let Event::Message { text } = event {
                shown = text;
            }
            Ok(())
        })
        .unwrap_err();
    assert!(error.contains("PDF annotation storage is unavailable"));
    assert_eq!(shown, "Answer remains.");
    let saved = Value::Array(agent.messages.lock().unwrap().clone()).encode();
    assert!(saved.contains("Answer remains."));
    assert!(!saved.contains("base64,"));
    fixture.finish();
}

#[test]
fn legacy_inline_pdf_image_is_hidden_from_history_reads() {
    let directory = Directory::new();
    let agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string("The report says hello.")),
        (
            "annotations",
            Value::Array(vec![Value::string("data:image/png;base64,AAAA")]),
        ),
    ]));
    let history = agent.execute_tool(&ToolCall {
        id: "history-read".into(),
        name: "read".into(),
        arguments: Value::object([("path", Value::string("history:1"))]).encode(),
    });
    let content = history.get("content").and_then(Value::as_str).unwrap();
    assert!(content.contains("The report says hello."));
    assert!(content.contains("[inline PDF image omitted]"));
    assert!(!content.contains("base64,"));
}
