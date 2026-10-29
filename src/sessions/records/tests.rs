use super::*;
use crate::{
    effort::Effort,
    test_support::{Directory, tool_call},
};

fn message(role: &str, content: &str) -> Value {
    Value::object([
        ("role", Value::string(role)),
        ("content", Value::string(content)),
    ])
}

fn local(after: usize, text: &str) -> Value {
    Value::object([
        ("type", Value::string("local_command")),
        ("command", Value::string("/fixture")),
        ("result", Value::string(text)),
        ("after_message", Value::number(after)),
    ])
}

#[test]
fn returned_turn_hides_reverted_compaction_but_keeps_unrelated_local_commands() {
    let directory = Directory::new();
    let mut doc = Document::new(
        directory.path().to_path_buf(),
        "fixture/model".into(),
        Effort::Default,
    )
    .unwrap();
    doc.messages = vec![
        message("system", "Fixture"),
        message("user", "Unanswered request"),
    ];
    let compaction = Value::object([
        ("type", Value::string("local_command")),
        ("command", Value::string("Context compaction")),
        ("result", Value::string("Reverted compaction")),
        ("after_message", Value::number(2)),
    ]);
    doc.events = vec![
        compaction,
        local(2, "Keep this command"),
        Value::object([
            ("type", Value::string("turn_returned")),
            ("from_message", Value::number(1)),
            ("after_message", Value::number(2)),
        ]),
    ];
    doc.context
        .restore_returned(&doc.events, doc.messages.len());
    super::super::context::validate(&doc).unwrap();
    let results: Vec<_> = doc
        .records()
        .into_iter()
        .filter_map(|record| match record {
            Record::Local { result, .. } => Some(result),
            _ => None,
        })
        .collect();
    assert_eq!(results, ["Keep this command"]);
    assert!(doc.events.iter().any(|event| event.get("result").and_then(Value::as_str) == Some("Reverted compaction")));
}

#[test]
fn records_keep_event_order_at_each_boundary_and_match_reused_tool_ids() {
    let directory = Directory::new();
    let mut doc = Document::new(
        directory.path().to_path_buf(),
        "fixture/model".into(),
        Effort::Default,
    )
    .unwrap();
    doc.messages.push(message("system", "Fixture"));
    doc.messages.push(message("user", "Request"));
    for name in ["read", "write"] {
        let legacy = tool_call("reused", name, Value::object([]));
        let call = if name == "write" {
            ToolCall::parse(&legacy).unwrap().value()
        } else {
            legacy
        };
        doc.messages.push(Value::object([
            ("role", Value::string("assistant")),
            ("tool_calls", Value::Array(vec![call])),
        ]));
        doc.messages.push(Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("reused")),
            ("content", Value::string("{}")),
        ]));
    }
    // Legacy validation allows events stored out of message-boundary order.
    doc.events = vec![
        local(6, "last"),
        local(1, "first"),
        local(4, "middle"),
        local(4, "same boundary"),
    ];
    super::super::context::validate(&doc).unwrap();
    let records = doc.records();
    for record in &records {
        match record {
            Record::Tool { id, result, .. } => {
                assert_eq!(id, "reused");
                assert!(*result == Value::object([]));
            }
            Record::Local { kind, .. } => assert_eq!(kind, "notice"),
            _ => {}
        }
    }
    let names = records
        .into_iter()
        .map(|record| match record {
            Record::Question { .. } => panic!("Unexpected question in local-command fixture"),
            Record::Collaboration { .. } => {
                panic!("Unexpected collaboration in local-command fixture")
            }
            Record::TurnStart => "turn start".into(),
            Record::Text { text, .. } => text,
            Record::Tool { name, .. } => name,
            Record::Local { result, .. } => result,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "first",
            "Request",
            "read",
            "middle",
            "same boundary",
            "write",
            "last"
        ]
    );
}

#[test]
fn large_histories_restore_all_messages_and_events_in_conversation_order() {
    let directory = Directory::new();
    let mut doc = Document::new(
        directory.path().to_path_buf(),
        "fixture/model".into(),
        Effort::Default,
    )
    .unwrap();
    doc.messages.push(message("system", "Fixture"));
    for index in 0..10000 {
        doc.events
            .push(local(doc.messages.len(), &format!("event {index}")));
        doc.messages
            .push(message("user", &format!("message {index}")));
    }
    let records = doc.records();
    assert_eq!(records.len(), 20000);
    for (index, pair) in records.chunks_exact(2).enumerate() {
        assert!(
            matches!(&pair[0], Record::Local { result, .. } if result == &format!("event {index}"))
        );
        assert!(
            matches!(&pair[1], Record::Text { text, .. } if text == &format!("message {index}"))
        );
    }
}
