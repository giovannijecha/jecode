use super::*;
use crate::{
    effort::Effort,
    json::{self, Value},
    redact::Redactor,
    test_support::{Directory, tool_call},
};
use std::sync::{Arc, Mutex};

fn document(directory: &Directory) -> Document {
    let mut doc = Document::new(
        directory.path().to_path_buf(),
        "fixture/model".into(),
        Effort::Default,
    )
    .unwrap();
    doc.messages = vec![
        Value::object([
            ("role", Value::string("system")),
            ("content", Value::string("Fixture")),
        ]),
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Run independent calls")),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::Null),
            (
                "tool_calls",
                Value::Array(vec![
                    tool_call("slow", "bash", Value::object([])),
                    tool_call("fast", "write", Value::object([])),
                    tool_call("later", "read", Value::object([])),
                ]),
            ),
        ]),
    ];
    doc.pending.active = true;
    doc
}

fn receipt(id: &str, result: Value, change: Value) -> Value {
    Value::object([
        ("id", Value::string(id)),
        ("result", result),
        (
            "timing",
            Value::object([
                ("type", Value::string("tool_timing")),
                ("total_ms", Value::number(12)),
            ]),
        ),
        ("change", change),
    ])
}

fn archive(doc: &Document, redactor: Redactor) -> crate::export::Archive {
    crate::export::Archive {
        backend: doc.backend.clone(),
        legacy_origin: None,
        model: doc.model.clone().unwrap(),
        directory: doc.directory.clone(),
        messages: Arc::new(Mutex::new(doc.messages.clone())),
        redactor,
        effort: doc.effort.name().into(),
        events: Arc::new(Mutex::new(doc.events.clone())),
    }
}

#[test]
fn completed_second_call_recovers_in_source_order_with_timing_and_change() {
    let home = Directory::new();
    let directory = Directory::new();
    let store = Store::new(home.path().to_path_buf(), directory.path()).unwrap();
    let mut doc = document(&directory);
    let change = crate::file_change::Change::between(Some("old\n"), "new\n").unwrap();
    doc.pending.batch = vec!["slow".into(), "fast".into()];
    doc.pending.completed = vec![receipt(
        "fast",
        Value::object([("bytes_written", Value::number(4))]),
        change.value(),
    )];
    let lease = store.create(&doc.id).unwrap();
    lease.save(&doc).unwrap();
    drop(lease);
    let (handle, _) = Handle::open(store, &doc.id, Redactor::empty()).unwrap();
    let recovered = handle.snapshot();
    let ids: Vec<_> = recovered.messages[3..]
        .iter()
        .map(|message| message.get("tool_call_id").and_then(Value::as_str).unwrap())
        .collect();
    assert_eq!(ids, ["slow", "fast", "later"]);
    let results: Vec<_> = recovered.messages[3..]
        .iter()
        .map(|message| {
            json::parse(message.get("content").and_then(Value::as_str).unwrap()).unwrap()
        })
        .collect();
    assert_eq!(
        results[0].get("outcome").and_then(Value::as_str),
        Some("unknown")
    );
    assert_eq!(results[1].get("bytes_written"), Some(&Value::number(4)));
    assert_eq!(
        results[2].get("outcome").and_then(Value::as_str),
        Some("not_executed")
    );
    assert!(recovered.records().iter().any(|record| matches!(
        record,
        Record::Tool { id, change: Some(actual), .. } if id == "fast" && actual == &change
    )));
    let timing = recovered
        .events
        .iter()
        .find(|event| event.get("type").and_then(Value::as_str) == Some("tool_timing"))
        .unwrap();
    assert_eq!(
        timing.get("tool_call_id").and_then(Value::as_str),
        Some("fast")
    );
    assert_eq!(timing.get("name").and_then(Value::as_str), Some("write"));
    assert_eq!(timing.get("after_message"), Some(&Value::number(5)));
    assert!(recovered.pending.batch.is_empty());
    assert!(recovered.pending.completed.is_empty());
}

#[test]
fn reverse_completion_order_recovers_both_known_results_without_reexecution() {
    let directory = Directory::new();
    let mut doc = document(&directory);
    doc.pending.batch = vec!["slow".into(), "fast".into()];
    doc.pending.completed = vec![
        receipt(
            "fast",
            Value::object([("value", Value::string("second"))]),
            Value::Null,
        ),
        receipt(
            "slow",
            Value::object([("value", Value::string("first"))]),
            Value::Null,
        ),
    ];
    let mut recovered = Document::parse(&doc.value()).unwrap();
    assert!(recovered.recover());
    let values: Vec<_> = recovered.messages[3..5]
        .iter()
        .map(|message| {
            json::parse(message.get("content").and_then(Value::as_str).unwrap())
                .unwrap()
                .get("value")
                .and_then(Value::as_str)
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(values, ["first", "second"]);
}

#[test]
fn batch_receipt_is_redacted_before_its_checkpoint() {
    let home = Directory::new();
    let directory = Directory::new();
    let store = Store::new(home.path().to_path_buf(), directory.path()).unwrap();
    let doc = document(&directory);
    let redactor = Redactor::new("fixture-secret".into());
    let archive = archive(&doc, redactor.clone());
    let handle = Handle::new(store, doc, redactor);
    let context = crate::context::Context::default();
    let routing = crate::backend::Routing::Default;
    handle
        .checkpoint(
            &archive,
            0,
            &context,
            &routing,
            true,
            Stage::ToolBatch(vec!["slow".into(), "fast".into()]),
        )
        .unwrap();
    handle
        .checkpoint(
            &archive,
            0,
            &context,
            &routing,
            true,
            Stage::ToolBatchResult(receipt(
                "fast",
                Value::object([("output", Value::string("fixture-secret"))]),
                Value::Null,
            )),
        )
        .unwrap();
    let saved = handle.store().fixture_load(&handle.id()).unwrap();
    assert!(
        !saved.pending.completed[0]
            .encode()
            .contains("fixture-secret")
    );
    assert_eq!(
        saved.pending.completed[0]
            .get("result")
            .and_then(|result| result.get("output"))
            .and_then(Value::as_str),
        Some("[redacted]")
    );
    assert_eq!(saved.pending.reads, ["slow", "fast"]);
}

#[test]
fn older_reader_treats_interrupted_explicit_batch_as_unknown() {
    let directory = Directory::new();
    let mut doc = document(&directory);
    doc.pending.batch = vec!["slow".into(), "fast".into()];
    doc.pending.reads = doc.pending.batch.clone();
    doc.pending.completed = vec![receipt(
        "fast",
        Value::object([("value", Value::string("known to new readers"))]),
        Value::Null,
    )];
    let mut old_view = doc.value();
    if let Value::Object(fields) = &mut old_view
        && let Some(Value::Object(pending)) = fields.get_mut("pending")
    {
        pending.remove("batch");
        pending.remove("completed");
    }
    let mut restored = Document::parse(&old_view).unwrap();
    assert!(restored.recover());
    for message in &restored.messages[3..5] {
        let result = json::parse(message.get("content").and_then(Value::as_str).unwrap()).unwrap();
        assert_eq!(
            result.get("outcome").and_then(Value::as_str),
            Some("unknown")
        );
    }
    let later = json::parse(
        restored.messages[5]
            .get("content")
            .and_then(Value::as_str)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        later.get("outcome").and_then(Value::as_str),
        Some("not_executed")
    );
}

#[cfg(windows)]
#[test]
fn failed_result_checkpoint_rolls_back_the_staged_receipt() {
    let home = Directory::new();
    let directory = Directory::new();
    let store = Store::new(home.path().to_path_buf(), directory.path()).unwrap();
    let doc = document(&directory);
    let redactor = Redactor::empty();
    let archive = archive(&doc, redactor.clone());
    let handle = Handle::new(store, doc, redactor);
    let context = crate::context::Context::default();
    let routing = crate::backend::Routing::Default;
    handle
        .checkpoint(
            &archive,
            0,
            &context,
            &routing,
            true,
            Stage::ToolBatch(vec!["slow".into(), "fast".into()]),
        )
        .unwrap();
    let journal = handle.store().journal_path(&handle.id());
    let original = std::fs::metadata(&journal).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&journal, readonly).unwrap();
    let error = handle
        .checkpoint(
            &archive,
            0,
            &context,
            &routing,
            true,
            Stage::ToolBatchResult(receipt("fast", Value::object([]), Value::Null)),
        )
        .unwrap_err();
    assert!(error.contains("read-only"), "{error}");
    assert!(handle.snapshot().pending.completed.is_empty());
    std::fs::set_permissions(&journal, original).unwrap();
    let saved = handle.store().fixture_load(&handle.id()).unwrap();
    assert!(saved.pending.completed.is_empty());
    assert_eq!(saved.pending.reads, ["slow", "fast"]);
}

#[test]
fn malformed_batch_and_older_pending_formats_are_distinguished() {
    let directory = Directory::new();
    let mut doc = document(&directory);
    doc.pending.batch = vec!["slow".into(), "fast".into()];
    let mut legacy = doc.value();
    if let Value::Object(fields) = &mut legacy
        && let Some(Value::Object(pending)) = fields.get_mut("pending")
    {
        pending.remove("batch");
        pending.remove("completed");
    }
    let parsed = Document::parse(&legacy).unwrap();
    assert!(parsed.pending.batch.is_empty() && parsed.pending.completed.is_empty());

    doc.pending.completed = vec![receipt("later", Value::object([]), Value::Null)];
    assert!(Document::parse(&doc.value()).is_err());
    doc.pending.completed = vec![receipt("fast", Value::string("wrong"), Value::Null)];
    assert!(Document::parse(&doc.value()).is_err());
    doc.pending.completed = vec![receipt("fast", Value::object([]), Value::Null); 2];
    assert!(Document::parse(&doc.value()).is_err());
    doc.pending.completed.clear();
    doc.pending.batch = vec!["slow".into(), "slow".into()];
    assert!(Document::parse(&doc.value()).is_err());
    doc.pending.batch = vec!["slow".into(), "fast".into()];
    doc.pending.reads = vec!["fast".into()];
    assert!(Document::parse(&doc.value()).is_err());
}
