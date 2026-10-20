use super::*;
use crate::{
    json,
    test_support::{Directory, HttpFixture, completion, tool_call},
};

const PDF: &[u8] = b"%PDF-1.4\n1 0 obj << /Type /Page >> endobj\n%%EOF\n";

fn read_call(id: &str, path: &str) -> Value {
    tool_call(id, "read", Value::object([("path", Value::string(path))]))
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

fn annotated_completion() -> Value {
    let annotation = r#""annotations":[{"type":"file","file":{"hash":"local-read-hash","name":"report.bin","content":[{"type":"text","text":"parsed report"}]}}],"#;
    json::parse(
        &completion("Reviewed both files.", vec![])
            .encode()
            .replacen("\"role\":", &format!("{annotation}\"role\":"), 1),
    )
    .unwrap()
}

#[test]
fn local_media_tool_receipts_feed_provider_and_survive_resume_without_sources() {
    let project = Directory::new();
    let home = Directory::new();
    let image = crate::attachments::tests::png(4, 3);
    let image_path = project.path().join("shot.bin");
    std::fs::write(&image_path, &image).unwrap();
    let first = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    read_call("image-read", "shot.bin"),
                    read_call("pdf-read", "tmp:report.bin"),
                ],
            ),
        ),
        (200, annotated_completion()),
    ]);
    let mut client = OpenRouter::fixture(first.endpoint.clone());
    client.fixture_inputs(&["text", "image", "file"]);
    let mut agent = Agent::new(client, Tools::new(project.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    let temp = agent.tools.temporary().unwrap().ensure().unwrap();
    let pdf_path = temp.join("report.bin");
    std::fs::write(&pdf_path, PDF).unwrap();
    agent
        .run_turn("Inspect the local files", &mut |_| Ok(()))
        .unwrap();
    let requests = first.finish();
    assert_eq!(requests.len(), 2);
    let messages = requests[1]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap();
    let shown = messages.last().unwrap();
    assert_eq!(shown.get("role").and_then(Value::as_str), Some("user"));
    assert_eq!(parts(shown), ["text", "image_url", "file"]);
    assert_eq!(
        requests[1]
            .body
            .get("plugins")
            .map(Value::encode)
            .as_deref(),
        Some(r#"[{"id":"file-parser","pdf":{"engine":"native"}}]"#),
    );

    let saved = agent.messages.lock().unwrap().clone();
    let receipts = saved
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .map(|message| {
            json::parse(message.get("content").and_then(Value::as_str).unwrap()).unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(receipts.len(), 2);
    let references = receipts
        .iter()
        .map(|receipt| {
            receipt
                .get("reference")
                .and_then(Value::as_str)
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        receipts[0].get("view").and_then(Value::as_str),
        Some("image")
    );
    assert_eq!(receipts[1].get("view").and_then(Value::as_str), Some("pdf"));
    let encoded = Value::Array(saved).encode();
    assert!(!encoded.contains("base64,"));
    assert!(!encoded.contains("iVBOR"));
    assert!(encoded.contains("local-read-hash"));
    assert!(
        agent
            .messages
            .lock()
            .unwrap()
            .iter()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
            .all(|message| message.get("attachments").is_none())
    );
    let export = agent.archive().save().unwrap();
    let archive = json::parse(&std::fs::read_to_string(&export).unwrap()).unwrap();
    let exported = archive
        .get("attachments")
        .and_then(Value::as_array)
        .unwrap();
    for reference in &references {
        let id = reference.strip_prefix("attachment:").unwrap();
        let asset = exported
            .iter()
            .find(|entry| entry.get("id").and_then(Value::as_str) == Some(id))
            .unwrap();
        let relative = asset.get("path").and_then(Value::as_str).unwrap();
        assert!(export.parent().unwrap().join(relative).exists());
    }
    let session_id = agent.sessions().unwrap().id();
    std::fs::remove_file(image_path).unwrap();
    agent.clean_temporary().unwrap();
    assert!(!pdf_path.exists());
    drop(agent);

    let store = crate::sessions::Store::new(home.path().to_path_buf(), project.path()).unwrap();
    let pool = store.attachments();
    let ids = references
        .iter()
        .map(|reference| reference.strip_prefix("attachment:").unwrap().to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        pool.collect(store.bucket(), &std::collections::BTreeSet::new(), &ids)
            .unwrap(),
        0,
        "the saved tool receipts must keep both assets reachable",
    );

    let second = HttpFixture::new(vec![
        (200, completion("Ready.", vec![])),
        (
            200,
            completion(
                "",
                vec![
                    read_call("image-again", &references[0]),
                    read_call("pdf-again", &references[1]),
                ],
            ),
        ),
        (200, completion("Both stored files are available.", vec![])),
    ]);
    let mut client = OpenRouter::fixture(second.endpoint.clone());
    client.fixture_inputs(&["text", "image", "file"]);
    let mut resumed = Agent::new(client, Tools::new(project.path()).unwrap());
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&session_id).unwrap();
    resumed
        .run_turn("Continue the analysis", &mut |_| Ok(()))
        .unwrap();
    let old_end = resumed.messages.lock().unwrap().len();
    // Model summaries can replace the old receipts in the request projection.
    // Their durable attachment IDs must still resolve from the session pool.
    resumed.context.from = old_end;
    resumed.context.summary = format!(
        "Local files remain at {} and {}",
        references[0], references[1]
    );
    resumed
        .run_turn("Show the files again", &mut |_| Ok(()))
        .unwrap();
    let requests = second.finish();
    let annotation = requests[0]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find_map(|message| message.get("annotations"))
        .unwrap()
        .encode();
    assert!(annotation.contains("local-read-hash"));
    let replay = requests[2]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(parts(replay.last().unwrap()), ["text", "image_url", "file"]);
    assert!(
        !Value::Array(resumed.messages.lock().unwrap().clone())
            .encode()
            .contains("base64,")
    );
}
