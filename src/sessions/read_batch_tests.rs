use super::*;
use crate::{
    effort::Effort,
    json::{self, Value},
    redact::Redactor,
    test_support::{Directory, tool_call},
};

fn document(directory: &Directory) -> Document {
    let mut doc = Document::new(
        directory.path().to_path_buf(),
        "fixture/model".into(),
        Effort::High,
    )
    .unwrap();
    doc.messages = vec![
        Value::object([
            ("role", Value::string("system")),
            ("content", Value::string("Fixture")),
        ]),
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Read files")),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("Working")),
        ]),
    ];
    doc
}

#[test]
fn interrupted_read_batch_recovers_each_started_read_and_preserves_order() {
    let home = Directory::new();
    let directory = Directory::new();
    let store = Store::new(home.path().to_path_buf(), directory.path()).unwrap();
    let mut doc = document(&directory);
    doc.messages.pop();
    doc.messages.push(Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::Null),
        (
            "tool_calls",
            Value::Array(vec![
                tool_call(
                    "read-a",
                    "read",
                    Value::object([("path", Value::string("a"))]),
                ),
                tool_call(
                    "read-b",
                    "read",
                    Value::object([("path", Value::string("b"))]),
                ),
                tool_call(
                    "later",
                    "write",
                    Value::object([
                        ("path", Value::string("must-not-exist")),
                        ("content", Value::string("no")),
                    ]),
                ),
            ]),
        ),
    ]));
    doc.pending = Pending {
        active: true,
        reads: vec!["read-a".into(), "read-b".into()],
        ..Pending::default()
    };
    let id = doc.id.clone();
    let lease = store.acquire(&id).unwrap();
    lease.save(&doc).unwrap();
    drop(lease);
    let (handle, _) = Handle::open(store, &id, Redactor::new("fixture-key".into())).unwrap();
    let recovered = handle.snapshot();
    assert_eq!(recovered.messages.len(), 6);
    for (message, (id, outcome)) in recovered.messages[3..].iter().zip([
        ("read-a", "unknown"),
        ("read-b", "unknown"),
        ("later", "not_executed"),
    ]) {
        assert_eq!(
            message.get("tool_call_id").and_then(Value::as_str),
            Some(id)
        );
        let result = json::parse(message.get("content").and_then(Value::as_str).unwrap()).unwrap();
        assert_eq!(result.get("outcome").and_then(Value::as_str), Some(outcome));
    }
    assert!(!recovered.pending.active);
    assert!(!directory.path().join("must-not-exist").exists());
    assert_eq!(
        recovered
            .records()
            .iter()
            .filter(|record| matches!(record, Record::Tool { .. }))
            .count(),
        3
    );
}

#[test]
fn saved_sessions_without_pending_reads_remain_loadable() {
    let directory = Directory::new();
    let mut value = document(&directory).value();
    if let Value::Object(fields) = &mut value
        && let Some(Value::Object(pending)) = fields.get_mut("pending")
    {
        pending.remove("reads");
    }
    let parsed = Document::parse(&value).unwrap();
    assert!(parsed.pending.reads.is_empty());
}
