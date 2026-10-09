use super::*;

fn ledger() -> Value {
    Value::object([
        ("objective", Value::string("Continue the user's long task")),
        (
            "constraints",
            Value::Array(vec![Value::string("Preserve every original test byte")]),
        ),
        (
            "remaining",
            Value::Array(vec![Value::string(
                "Run the still outstanding compatibility check",
            )]),
        ),
        (
            "next_action",
            Value::string("Run the outstanding check, preserving earlier work"),
        ),
        (
            "completed",
            Value::Array(
                (0..250)
                    .map(|index| {
                        Value::object([
                            (
                                "description",
                                Value::string(format!(
                                    "Finished stage {index}: {}",
                                    "exact original details; ".repeat(10)
                                )),
                            ),
                            ("kind", Value::string("inspection")),
                            (
                                "evidence",
                                Value::Array(vec![Value::string(format!("history:{index}"))]),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

#[test]
fn completed_proof_growth_does_not_remove_constraints_or_open_work_from_the_view() {
    let original = ledger();
    let text = original.encode();
    let view = active_view(&text, 2400);
    assert!(text.len() > 50000);
    assert!(view.len() <= 2400);
    let view = json::parse(&view).unwrap();
    for field in ["objective", "constraints", "remaining", "next_action"] {
        assert_eq!(view.get(field), original.get(field));
    }
    let archive = view.get("completed_archive").unwrap();
    assert_eq!(archive.get("count").and_then(Value::as_usize), Some(250));
    assert_eq!(
        archive.get("reference").and_then(Value::as_str),
        Some("history:memory")
    );
    assert!(archive.get("omitted").and_then(Value::as_usize).unwrap() > 200);
    assert!(
        view.get("completed")
            .unwrap()
            .encode()
            .contains("history:249")
    );
    assert_eq!(original.encode(), text);
}

#[test]
fn projection_refuses_to_hide_genuinely_oversized_constraints() {
    let mut original = ledger();
    if let Value::Object(fields) = &mut original {
        fields.insert(
            "constraints".into(),
            Value::Array(vec![Value::string("keep exactly ".repeat(2000))]),
        );
    }
    let view = active_view(&original.encode(), 2400);
    assert!(view.len() > 2400);
    assert_eq!(
        json::parse(&view).unwrap().get("constraints"),
        original.get("constraints")
    );
}

#[test]
fn a_saved_projection_budget_keeps_the_complete_native_ledger() {
    let text = ledger().encode();
    let context = crate::context::Context {
        summary: text.clone(),
        memory_limit: Some(2400),
        ..Default::default()
    };
    let restored = crate::context::Context::parse(Some(&context.value())).unwrap();
    assert_eq!(restored.summary, text);
    assert_eq!(restored.memory_limit, Some(2400));
    assert!(restored.memory_view().len() <= 2400);
    let mut legacy = context.value();
    if let Value::Object(fields) = &mut legacy {
        fields.remove("memory_limit");
    }
    assert_eq!(
        crate::context::Context::parse(Some(&legacy))
            .unwrap()
            .memory_limit,
        None
    );
}

#[test]
fn copying_a_shortened_preview_cannot_shorten_the_durable_completed_identity() {
    let description = "Exact earlier inspection details. ".repeat(30);
    let entry = Value::object([
        ("description", Value::string(&description)),
        ("kind", Value::string("inspection")),
        ("evidence", Value::Array(vec![Value::string("history:0")])),
    ]);
    let mut previous = ledger();
    if let Value::Object(fields) = &mut previous {
        fields.insert("completed".into(), Value::Array(vec![entry.clone()]));
    }
    let mut candidate = previous.clone();
    if let Value::Object(fields) = &mut candidate {
        let mut shortened = entry;
        if let Value::Object(fields) = &mut shortened {
            fields.insert(
                "description".into(),
                Value::string(super::super::shorten(&description, 64)),
            );
        }
        fields.insert("completed".into(), Value::Array(vec![shortened]));
    }
    let messages = vec![Value::object([
        ("role", Value::string("tool")),
        (
            "content",
            Value::string("{\"content\":\"original inspected text\"}"),
        ),
    ])];
    let retained =
        super::super::prepare(&candidate.encode(), &previous.encode(), &messages, 1, 700).unwrap();
    assert_eq!(retained, previous.encode());
    assert!(active_view(&retained, 700).len() <= 700);
}

#[test]
fn the_newest_evidence_stays_visible_when_native_carry_appends_older_entries() {
    let mut original = ledger();
    if let Value::Object(fields) = &mut original
        && let Some(Value::Array(entries)) = fields.get_mut("completed")
    {
        entries.reverse();
    }
    let view = json::parse(&active_view(&original.encode(), 2400)).unwrap();
    assert!(
        view.get("completed")
            .unwrap()
            .encode()
            .contains("history:249")
    );
}

#[test]
fn summary_input_separates_native_completed_proof_from_new_additions() {
    let original = ledger();
    let text = original.encode();
    let view = json::parse(&summary_input(&text, 2400)).unwrap();
    assert_eq!(view.get("completed"), Some(&Value::Array(vec![])));
    for field in ["objective", "constraints", "remaining", "next_action"] {
        assert_eq!(view.get(field), original.get(field));
    }
    let archive = view.get("completed_archive").unwrap();
    assert_eq!(archive.get("count"), Some(&Value::number(250)));
    let retained = archive.get("retained").unwrap().as_array().unwrap();
    assert!(!retained.is_empty());
    for entry in retained {
        let old = original
            .get("completed")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .find(|old| old.get("evidence") == entry.get("evidence"))
            .unwrap();
        assert_eq!(entry.get("kind"), old.get("kind"));
        assert_eq!(entry.get("evidence"), old.get("evidence"));
    }
    assert_eq!(original.encode(), text);
}

#[test]
fn a_copied_read_only_archive_preserves_the_full_native_completed_ledger() {
    let mut previous = ledger();
    let description = "Exact inspected source details. ".repeat(30);
    let entry = Value::object([
        ("description", Value::string(&description)),
        ("kind", Value::string("inspection")),
        ("evidence", Value::Array(vec![Value::string("history:0")])),
    ]);
    if let Value::Object(fields) = &mut previous {
        fields.insert("completed".into(), Value::Array(vec![entry.clone()]));
    }
    let candidate = summary_input(&previous.encode(), 700);
    let messages = vec![Value::object([
        ("role", Value::string("tool")),
        (
            "content",
            Value::string(r#"{"content":"original inspected source"}"#),
        ),
    ])];
    let kept = super::super::prepare(&candidate, &previous.encode(), &messages, 1, 700).unwrap();
    let kept = json::parse(&kept).unwrap();
    assert_eq!(kept.get("completed"), Some(&Value::Array(vec![entry])));
    assert!(kept.get("completed_archive").is_none());
}
