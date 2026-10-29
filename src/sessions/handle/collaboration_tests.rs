use super::*;
use crate::{
    backend::Routing,
    effort::Effort,
    export::Archive,
    json::Value,
    test_support::{Directory, tool_call},
};

#[test]
fn asynchronous_receipt_saves_before_unsaved_tool_result_without_corrupting_pending_state() {
    let home = Directory::new();
    let project = Directory::new();
    let store = Store::new(home.path().to_path_buf(), project.path()).unwrap();
    let mut document = Document::new(
        project.path().to_path_buf(),
        "fixture/model".into(),
        Effort::High,
    )
    .unwrap();
    for (role, content) in [("system", "system"), ("user", "request")] {
        document.messages.push(Value::object([
            ("role", Value::string(role)),
            ("content", Value::string(content)),
        ]));
    }
    let redactor = Redactor::empty();
    let archive = Archive {
        backend: "openrouter".into(),
        legacy_origin: None,
        model: "fixture/model".into(),
        directory: project.path().to_path_buf(),
        messages: Arc::new(Mutex::new(document.messages.clone())),
        redactor: redactor.clone(),
        effort: "high".into(),
        events: Arc::new(Mutex::new(vec![])),
    };
    let handle = Handle::new(store.clone(), document, redactor.clone());
    let id = handle.id();
    let checkpoint = |stage| {
        handle.checkpoint(
            &archive,
            0,
            &crate::context::Context::default(),
            &Routing::Default,
            true,
            stage,
        )
    };
    checkpoint(Stage::Begin).unwrap();
    archive.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::Null),
        (
            "tool_calls",
            Value::Array(vec![tool_call(
                "c1",
                "read",
                Value::object([("path", Value::string("file.txt"))]),
            )]),
        ),
    ]));
    checkpoint(Stage::Completion).unwrap();
    checkpoint(Stage::Tool("c1".into())).unwrap();
    archive.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("tool")),
        ("tool_call_id", Value::string("c1")),
        ("content", Value::string("{}")),
    ]));
    archive.events.lock().unwrap().push(Value::object([
        ("type", Value::string("tool_timing")),
        ("after_message", Value::number(4)),
    ]));
    let receipt = handle
        .append_event(
            &archive,
            Value::object([
                ("type", Value::string("collaboration")),
                ("kind", Value::string("returned")),
                ("sequence", Value::number(1)),
                ("records", Value::Array(vec![])),
                ("after_message", Value::number(4)),
            ]),
        )
        .unwrap();
    assert_eq!(
        receipt.get("after_message").and_then(Value::as_usize),
        Some(3)
    );
    let saved = store.fixture_load(&id).unwrap();
    assert_eq!(saved.messages.len(), 3);
    assert_eq!(saved.pending.tool.as_deref(), Some("c1"));
    assert_eq!(saved.events.len(), 1);
    checkpoint(Stage::ToolFinished).unwrap();
    checkpoint(Stage::Ready).unwrap();
    let saved = store.fixture_load(&id).unwrap();
    assert_eq!(saved.messages.len(), 4);
    assert_eq!(saved.events.len(), 2);
    assert_eq!(
        saved.events[0].get("type").and_then(Value::as_str),
        Some("collaboration")
    );
    assert_eq!(
        saved.events[1].get("type").and_then(Value::as_str),
        Some("tool_timing")
    );
    drop(handle);
    let (resumed, _) = Handle::open(store, &id, redactor).unwrap();
    assert_eq!(resumed.snapshot().events.len(), 2);
}

#[test]
fn closed_collaborator_receipt_survives_journal_reload() {
    let home = Directory::new();
    let project = Directory::new();
    let store = Store::new(home.path().to_path_buf(), project.path()).unwrap();
    let mut document = Document::new(
        project.path().to_path_buf(),
        "fixture/model".into(),
        Effort::High,
    )
    .unwrap();
    document.messages.push(Value::object([
        ("role", Value::string("system")),
        ("content", Value::string("fixture")),
    ]));
    let redactor = Redactor::empty();
    let archive = Archive {
        backend: "openrouter".into(),
        legacy_origin: None,
        model: "fixture/model".into(),
        directory: project.path().to_path_buf(),
        messages: Arc::new(Mutex::new(document.messages.clone())),
        redactor: redactor.clone(),
        effort: "high".into(),
        events: Arc::new(Mutex::new(vec![])),
    };
    let handle = Handle::new(store.clone(), document, redactor.clone());
    let id = handle.id();
    handle
        .checkpoint(
            &archive,
            0,
            &crate::context::Context::default(),
            &Routing::Default,
            false,
            Stage::Ready,
        )
        .unwrap();
    let closed = Value::object([
        ("type", Value::string("collaboration")),
        ("kind", Value::string("closed")),
        ("id", Value::string("a24")),
        ("number", Value::number(24)),
        ("sequence", Value::number(96)),
        ("version", Value::number(2)),
        ("records", Value::Array(vec![])),
        ("after_message", Value::number(1)),
    ]);
    let saved = handle.append_event(&archive, closed).unwrap();
    drop(handle);
    let (resumed, _) = Handle::open(store, &id, redactor).unwrap();
    assert_eq!(resumed.snapshot().events, vec![saved]);
}
