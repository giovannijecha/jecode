use super::*;
use crate::agent::Agent;
use crate::json;
use crate::openrouter::OpenRouter;
use crate::test_support::Directory;
use crate::tools::Tools;

#[test]
fn exports_exact_protocol_messages_and_masks_the_key_in_nested_arguments() {
    let directory = Directory::new();
    let arguments = Value::object([("command", Value::string("echo fixture-secret"))]);
    let original = vec![
        Value::object([
            ("role", Value::string("assistant")),
            (
                "reasoning_details",
                Value::Array(vec![Value::string("opaque reasoning")]),
            ),
            (
                "tool_calls",
                Value::Array(vec![Value::object([
                    ("id", Value::string("call-1")),
                    ("arguments", Value::string(arguments.encode())),
                ])]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("call-1")),
            (
                "content",
                Value::string("{\"stdout\":\"hello\\n\",\"truncated\":true}"),
            ),
        ]),
    ];
    let archive = Archive {
        effort: "default".into(),
        events: Arc::new(Mutex::new(vec![])),
        attachments: None,
        model: "fixture/model".into(),
        directory: directory.path().to_path_buf(),
        messages: Arc::new(Mutex::new(original.clone())),
        redactor: Redactor::new("fixture-secret".into()),
    };
    let first = archive.save().unwrap();
    let second = archive.save().unwrap();
    assert_ne!(first, second);
    assert_eq!(first.parent(), Some(directory.path()));
    let text = fs::read_to_string(first).unwrap();
    assert!(!text.contains("fixture-secret"));
    let document = json::parse(&text).unwrap();
    assert_eq!(document.get("format_version"), Some(&Value::number(1)));
    assert_eq!(
        document.get("messages"),
        Some(&Value::Array(
            archive
                .redactor
                .value(&Value::Array(original.clone()))
                .as_array()
                .unwrap()
                .to_vec()
        ))
    );
    assert_eq!(*archive.messages.lock().unwrap(), original);
    assert!(text.contains("opaque reasoning"));
}

#[test]
fn an_existing_archive_observes_clear_and_export_errors_are_recoverable() {
    let directory = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    let archive = agent.archive();
    archive
        .messages
        .lock()
        .unwrap()
        .push(Value::string("previous conversation"));
    agent.clear();
    assert_eq!(
        archive
            .document()
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        !fs::read_to_string(archive.save().unwrap())
            .unwrap()
            .contains("previous conversation")
    );
    let mut unavailable = archive.clone();
    unavailable.directory = directory.path().join("missing");
    assert!(unavailable.save().unwrap_err().contains("Could not create"));
    assert!(archive.save().is_ok());
}

#[test]
fn exports_bundle_referenced_attachments_beside_the_json() {
    let directory = Directory::new();
    let storage = Directory::new();
    let pool = crate::attachments::Pool::new(storage.path().join("attachments"));
    let blob = pool.import_bytes("data.bin", &[0, 255, 7]).unwrap();
    let parsed_page = pool
        .import_bytes("pdf-page.png", &[137, 80, 78, 71])
        .unwrap();
    let unused = pool.import_bytes("unused.txt", b"x").unwrap();
    let prompt = crate::attachments::Prompt::new(
        format!("{0} and again {0}", crate::attachments::MARKER),
        vec![blob.clone(), blob.clone()],
    );
    let archive = Archive {
        effort: "default".into(),
        events: Arc::new(Mutex::new(vec![])),
        attachments: Some(pool),
        model: "fixture/model".into(),
        directory: directory.path().to_path_buf(),
        messages: Arc::new(Mutex::new(vec![
            prompt.message(),
            Value::object([
                ("role", Value::string("assistant")),
                (
                    "annotations",
                    Value::Array(vec![Value::object([(
                        "file",
                        Value::object([
                            ("hash", Value::string("parsed-hash")),
                            (
                                "content",
                                Value::object([
                                    ("jecode_attachment", Value::string(parsed_page.reference())),
                                    ("media", Value::string("image/png")),
                                ]),
                            ),
                        ]),
                    )])]),
                ),
            ]),
        ])),
        redactor: Redactor::new("fixture-secret".into()),
    };
    let path = archive.save().unwrap();
    let document = json::parse(&fs::read_to_string(&path).unwrap()).unwrap();
    let listed = document
        .get("attachments")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(listed.len(), 2);
    let relative = listed[0].get("path").and_then(Value::as_str).unwrap();
    assert_eq!(
        fs::read(path.parent().unwrap().join(relative)).unwrap(),
        [0, 255, 7]
    );
    assert!(relative.ends_with(&format!("{}/data.bin", blob.id)));
    let parsed = listed[1].get("path").and_then(Value::as_str).unwrap();
    assert_eq!(
        fs::read(path.parent().unwrap().join(parsed)).unwrap(),
        [137, 80, 78, 71]
    );
    assert!(parsed.ends_with(&format!("{}/pdf-page.png", parsed_page.id)));
    assert!(!bundle_directory(&path).join(&unused.id).exists());
    // Without storage the export fails whole instead of dropping data.
    let orphan = Archive {
        attachments: None,
        ..archive
    };
    assert!(orphan.save().is_err());
    assert_eq!(
        fs::read_dir(directory.path()).unwrap().count(),
        2,
        "only the first export and its folder remain"
    );
}

#[test]
fn export_bundles_attachment_read_from_another_session() {
    let directory = Directory::new();
    let storage = Directory::new();
    let pool = crate::attachments::Pool::new(storage.path().join("attachments"));
    let png = crate::attachments::tests::png(4, 3);
    let image = pool.import_bytes("other-session.png", &png).unwrap();
    let result = Value::object([
        ("view", Value::string("image")),
        ("attachment", image.value()),
        ("reference", Value::string(image.reference())),
    ]);
    let archive = Archive {
        effort: "default".into(),
        events: Arc::new(Mutex::new(vec![])),
        attachments: Some(pool),
        model: "fixture/model".into(),
        directory: directory.path().to_path_buf(),
        messages: Arc::new(Mutex::new(vec![Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("read-other")),
            ("content", Value::string(result.encode())),
        ])])),
        redactor: Redactor::new("fixture-secret".into()),
    };
    let path = archive.save().unwrap();
    let document = json::parse(&fs::read_to_string(&path).unwrap()).unwrap();
    let listed = document
        .get("attachments")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].get("id").and_then(Value::as_str),
        Some(image.id.as_str())
    );
    let relative = listed[0].get("path").and_then(Value::as_str).unwrap();
    assert_eq!(
        fs::read(path.parent().unwrap().join(relative)).unwrap(),
        png
    );
    *archive.messages.lock().unwrap() = vec![Value::object([
        ("role", Value::string("tool")),
        (
            "content",
            Value::string(format!("Saw {} in plain output", image.reference())),
        ),
    ])];
    let plain = archive.save().unwrap();
    let plain_document = json::parse(&fs::read_to_string(&plain).unwrap()).unwrap();
    assert!(plain_document.get("attachments").is_none());
    assert!(!bundle_directory(&plain).exists());
}
