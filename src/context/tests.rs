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
        Some(&Value::object([("prompt_tokens", Value::number(1000))])),
        2,
    );
    context.calibrate(6000);
    assert_eq!(context.estimate_bytes(12000), 6000);
    assert_eq!(context.estimate_bytes(1), 1);
    let restored = Context::parse(Some(&context.value())).unwrap();
    assert_eq!(restored.calibration, context.calibration);
    context.reset_usage();
    assert_eq!(context.estimate_bytes(12000), 12000);
    context.observe(
        Some(&Value::object([("prompt_tokens", Value::number(4000))])),
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
    assert_eq!(context.estimate_bytes(18000), 9000);
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
fn measured_history_uses_calibrated_growth_and_falls_back_without_density() {
    let mut messages = vec![message("system", "Agent"), message("user", "Inspect")];
    let mut context = Context::default();
    context.observe(
        Some(&Value::object([("prompt_tokens", Value::number(1000))])),
        2,
    );
    context.calibrate(6000);
    let appended = message("tool", &"new output ".repeat(1000));
    let size = appended.encode().len();
    messages.push(appended);
    assert_eq!(context.estimate(&messages, 0), 1000 + size.div_ceil(2));
    context.calibration = None;
    assert_eq!(context.estimate(&messages, 0), 1000 + size);
}

#[test]
fn missing_or_invalid_usage_cannot_recalibrate_a_larger_request_with_old_tokens() {
    let missing = Value::object([("completion_tokens", Value::number(20))]);
    let zero = Value::object([("prompt_tokens", Value::number(0))]);
    for usage in [None, Some(&missing), Some(&zero)] {
        let mut context = Context::default();
        context.observe(
            Some(&Value::object([("prompt_tokens", Value::number(1000))])),
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
    // The oldest requests leave the view; history:requests still returns them.
    assert!(!encoded.contains("Original objective"));
    assert!(encoded.contains("Earlier requests are omitted"));
    assert!(encoded.contains("history:requests"));
}
