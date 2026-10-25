use super::*;

fn message(role: &str, text: &str) -> Value {
    Value::object([
        ("role", Value::string(role)),
        ("content", Value::string(text)),
    ])
}

#[test]
fn a_compacted_view_always_contains_the_users_request() {
    let messages = vec![
        message("system", "Agent"),
        message(
            "user",
            "Repair the parser; preserve tests and the full i64 range.",
        ),
    ];
    let context = Context {
        from: messages.len(),
        summary: "Implementation complete; verify next.".into(),
        ..Context::default()
    };
    let view = context.project(&messages, 0);
    assert!(view.iter().any(
        |item| item.get("role").and_then(Value::as_str) == Some("user")
            && item.encode().contains("preserve tests")
    ));
    assert_eq!(messages.len(), 2);
}

#[test]
fn latest_request_keeps_its_middle_when_earlier_requests_fill_the_preview() {
    let mut messages = vec![message("system", "Agent")];
    for _ in 0..49 {
        messages.push(message("user", "Earlier task ".repeat(100).as_str()));
    }
    let latest = format!(
        "{} CURRENT GOAL: inspect only; preserve the implementation exactly. {}",
        "Context ".repeat(100),
        "Details ".repeat(100),
    );
    messages.push(message("user", &latest));
    let context = Context {
        from: messages.len(),
        summary: "Earlier feature work; next action: edit the implementation.".into(),
        preview_limit: 2048,
        ..Context::default()
    };
    let original = messages.clone();
    let request = context.user_requests(&messages);
    assert!(
        request.contains(&latest),
        "latest request was needlessly excerpted"
    );
    assert!(request.contains("history:50"));
    assert!(request.contains("history:requests"));
    assert_eq!(messages, original);
    let view = context.project(&messages, 0);
    assert!(
        view.iter()
            .any(|value| value.encode().contains("CURRENT GOAL"))
    );
}

#[test]
fn calibrated_projection_uses_a_margin_and_resets_on_model_change() {
    let mut context = Context::default();
    assert_eq!(context.estimate_bytes(12000), 12000);
    context.observe(
        Some(&Usage {
            input_tokens: Some(1000),
            ..Usage::default()
        }),
        2,
    );
    context.calibrate(6000);
    assert_eq!(context.estimate_bytes(12000), 3000);
    assert_eq!(context.estimate_bytes(1), 1);
    let restored = Context::parse(Some(&context.value())).unwrap();
    assert_eq!(restored.calibration, context.calibration);
    context.reset_usage();
    assert_eq!(context.estimate_bytes(12000), 12000);
    context.observe(
        Some(&Usage {
            input_tokens: Some(4000),
            ..Usage::default()
        }),
        2,
    );
    context.calibrate(6000);
    assert_eq!(context.estimate_bytes(12000), 12000);
}

#[test]
fn summary_byte_budgets_keep_a_separate_margin_and_safe_fallbacks() {
    let mut context = Context::default();
    assert_eq!(context.summary_byte_budget(6000), 6000);
    context.calibration = Some((1000, 6000));
    assert_eq!(context.summary_byte_budget(6000), 18000);
    assert_eq!(context.estimate_bytes(18000), 4500);
    context.calibration = Some((2000, 7000));
    for budget in [0, 1, 17, 6000] {
        let bytes = context.summary_byte_budget(budget);
        assert!((bytes * 4000).div_ceil(7000) <= budget);
        assert!(((bytes + 1) * 4000).div_ceil(7000) > budget);
    }
    context.calibration = Some((4000, 6000));
    assert_eq!(context.summary_byte_budget(6000), 6000);
    context.calibration = Some((1, usize::MAX));
    assert_eq!(context.summary_byte_budget(usize::MAX), usize::MAX);
    context.reset_usage();
    assert_eq!(context.summary_byte_budget(6000), 6000);
}

#[test]
fn opaque_native_fields_do_not_consume_portable_context_budget() {
    let mut assistant = message("assistant", "Short answer");
    if let Value::Object(fields) = &mut assistant {
        fields.insert(
            "encrypted_reasoning".into(),
            Value::string("x".repeat(100_000)),
        );
    }
    crate::conversation::stamp(
        &mut assistant,
        &crate::conversation::Origin::new("openrouter", "original-model"),
    );
    assert_eq!(
        bytes(&[assistant.clone()]),
        portable(&assistant).encode().len()
    );
    assert!(assistant.encode().len() > bytes(&[assistant]) + 100_000);
}

#[test]
fn model_switch_keeps_stamped_history_for_return_to_original_model() {
    let mut original = message("assistant", "First model answer");
    if let Value::Object(fields) = &mut original {
        fields.insert("reasoning_details".into(), Value::string("opaque"));
    }
    crate::conversation::stamp(
        &mut original,
        &crate::conversation::Origin::new("openrouter", "model-a"),
    );
    let legacy = Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string("Legacy answer")),
        ("reasoning_details", Value::string("legacy opaque")),
    ]);
    let context = Context::default();
    assert_eq!(context.project_message(&original, 1, 3), original);
    let projected_legacy = context.project_message(&legacy, 2, 3);
    assert!(projected_legacy.get("reasoning_details").is_none());
    assert_eq!(projected_legacy.get("content"), legacy.get("content"));
}

#[test]
fn missing_or_invalid_usage_cannot_recalibrate_a_larger_request_with_old_tokens() {
    let missing = Usage {
        output_tokens: Some(20),
        ..Usage::default()
    };
    let zero = Usage {
        input_tokens: Some(0),
        ..Usage::default()
    };
    for usage in [None, Some(&missing), Some(&zero)] {
        let mut context = Context::default();
        context.observe(
            Some(&Usage {
                input_tokens: Some(1000),
                ..Usage::default()
            }),
            2,
        );
        context.calibrate(6000);
        context.observe(usage, 4);
        context.calibrate(20000);
        assert_eq!(context.input_tokens, None);
        assert_eq!(context.measured_end, 0);
        assert_eq!(context.calibration, None);
        assert_eq!(context.estimate_bytes(20000), 20000);
    }
}

#[test]
fn earlier_instructions_and_later_corrections_survive_together() {
    let messages = vec![
        message("system", "Agent"),
        message(
            "user",
            "Preserve protected.txt and use exact decimal arithmetic.",
        ),
        message("assistant", "Changed the parser."),
        message("user", "Now add an exclusive upper date bound."),
    ];
    let context = Context {
        from: messages.len(),
        summary: "Next: run checks.".into(),
        ..Context::default()
    };
    let view = context.project(&messages, 0);
    let users = view
        .iter()
        .filter(|item| item.get("role").and_then(Value::as_str) == Some("user"))
        .map(Value::encode)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(users.contains("protected.txt"));
    assert!(users.contains("exclusive upper date bound"));
}

#[test]
fn a_request_already_in_recent_history_is_not_duplicated() {
    let messages = vec![
        message("system", "Agent"),
        message("user", "Current request"),
    ];
    assert_eq!(Context::default().project(&messages, 0), messages);
}

#[test]
fn recent_requests_stay_verbatim_newest_first_within_the_budget() {
    let mut messages = vec![
        message("system", "Agent"),
        message("user", "Original objective: preserve protected.txt"),
    ];
    for index in 0..30 {
        messages.push(message(
            "user",
            &format!("Correction {index}: {}", "detail ".repeat(70)),
        ));
    }
    messages.push(message(
        "user",
        "Latest correction: exclusive upper date bound",
    ));
    let context = Context {
        from: messages.len(),
        summary: "Continue the current change.".into(),
        ..Context::default()
    };
    let view = context.project(&messages, 0);
    let encoded = view
        .iter()
        .map(Value::encode)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(encoded.contains("Latest correction: exclusive upper date bound"));
    assert!(encoded.contains(&format!("Correction 29: {}", "detail ".repeat(70))));
    // Keep the original goal beside recent corrections; middle requests can leave the view.
    assert!(encoded.contains("Original objective: preserve protected.txt"));
    assert!(!encoded.contains("Correction 0:"));
    assert!(encoded.contains("Earlier requests are omitted"));
    assert!(encoded.contains("history:requests"));
}

#[test]
fn the_displayed_percentage_reaches_the_threshold_exactly_with_the_budget() {
    assert_eq!(threshold(24000, 80), 19200);
    assert_eq!(threshold(1001, 80), 801);
    assert_eq!(threshold(1, 1), 1);
    assert!(threshold(usize::MAX, 99) < usize::MAX);
    for (capacity, percent) in [(24000, 80), (1001, 80), (128_000, 1), (7, 99)] {
        let budget = threshold(capacity, percent);
        let status = |tokens| Status {
            tokens,
            capacity: Some(capacity),
            auto_compact: percent,
        };
        assert!(status(budget).percent() >= Some(usize::from(percent)));
        assert!(status(budget - 1).percent() < Some(usize::from(percent)));
    }
    let unknown = Status {
        tokens: 5000,
        capacity: None,
        auto_compact: 80,
    };
    assert_eq!(unknown.percent(), None);
    assert_eq!(
        Status {
            capacity: Some(0),
            ..unknown
        }
        .percent(),
        None
    );
    for (value, valid) in [
        (0, false),
        (1, true),
        (99, true),
        (100, false),
        (356, false),
    ] {
        assert_eq!(auto_compact_percent(value).is_ok(), valid, "{value}");
    }
}

#[test]
fn the_effective_capacity_includes_a_learned_ceiling() {
    let limits = Limits {
        context: 24000,
        output: Some(4096),
    };
    let mut context = Context::default();
    assert_eq!(context.capacity(None), None);
    assert_eq!(context.capacity(Some(limits)), Some(24000));
    assert_eq!(context.budget(limits, 80), 19200);
    context.ceiling = Some(12000);
    assert_eq!(context.capacity(None), Some(12000));
    assert_eq!(context.capacity(Some(limits)), Some(12000));
    assert_eq!(context.budget(limits, 50), 6000);
    context.ceiling = Some(30000);
    assert_eq!(context.capacity(Some(limits)), Some(24000));
}
