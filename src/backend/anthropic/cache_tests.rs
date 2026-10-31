use super::*;

#[test]
fn cache_keeps_system_prefix_and_bridges_long_history() {
    let mut messages = vec![Value::object([
        ("role", Value::string("system")),
        ("content", Value::string("Stable project instructions")),
    ])];
    for index in 0..44 {
        messages.push(Value::object([
            (
                "role",
                Value::string(if index % 2 == 0 { "user" } else { "assistant" }),
            ),
            ("content", Value::string(format!("Turn {index}"))),
        ]));
    }
    let request = wire::request(
        "model-a",
        Effort::Default,
        &messages,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        request.get("cache_control"),
        Some(&Value::object([("type", Value::string("ephemeral"))]))
    );
    let system = request.get("system").and_then(Value::as_array).unwrap();
    assert_eq!(
        system[0].get("text").and_then(Value::as_str),
        Some("You are Claude Code, Anthropic's official CLI for Claude.")
    );
    assert!(system[1].get("cache_control").is_some());
    let history = request.get("messages").and_then(Value::as_array).unwrap();
    let marked: Vec<_> = history
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            message
                .get("content")
                .and_then(Value::as_array)
                .filter(|blocks| {
                    blocks
                        .iter()
                        .any(|block| block.get("cache_control").is_some())
                })
                .map(|_| index)
        })
        .collect();
    assert_eq!(marked, [40, 42]);
    assert_eq!(
        history
            .last()
            .unwrap()
            .get("content")
            .and_then(Value::as_array)
            .unwrap()[0]
            .get("text")
            .and_then(Value::as_str),
        Some("Turn 43")
    );
}

#[test]
fn prior_tool_batch_boundary_survives_a_new_large_batch() {
    let mut messages = vec![Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Start")),
    ])];
    for batch in 0..2 {
        messages.push(Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("Use tools")),
        ]));
        for index in 0..24 {
            messages.push(Value::object([
                ("role", Value::string("tool")),
                (
                    "tool_call_id",
                    Value::string(format!("toolu_{batch}_{index}")),
                ),
                ("content", Value::string(format!("Result {index}"))),
            ]));
        }
        if batch == 0 {
            let first = wire::request(
                "model-a",
                Effort::Default,
                &messages,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let first_history = first.get("messages").and_then(Value::as_array).unwrap();
            let first_results = first_history[2]
                .get("content")
                .and_then(Value::as_array)
                .unwrap();
            assert!(first_results.last().unwrap().get("cache_control").is_some());
        }
    }
    messages.push(Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Changing state")),
        ("jecode_request_state", Value::Bool(true)),
    ]));
    let request = wire::request(
        "model-a",
        Effort::Default,
        &messages,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    assert!(request.get("cache_control").is_none());
    let history = request.get("messages").and_then(Value::as_array).unwrap();
    for index in [2, 4] {
        let blocks = history[index]
            .get("content")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(blocks.len(), 24);
        assert!(blocks.last().unwrap().get("cache_control").is_some());
    }
    assert!(history[5].get("content").and_then(Value::as_str).is_some());
}

#[test]
fn request_state_suffix_is_outside_the_cached_prefix() {
    let messages = [
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Durable question")),
        ]),
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Changing state")),
            ("jecode_request_state", Value::Bool(true)),
        ]),
    ];
    let request = wire::request(
        "model-a",
        Effort::Default,
        &messages,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    assert!(request.get("cache_control").is_none());
    let history = request.get("messages").and_then(Value::as_array).unwrap();
    assert!(
        history[0].get("content").and_then(Value::as_array).unwrap()[0]
            .get("cache_control")
            .is_some()
    );
    assert_eq!(
        history[1].get("content"),
        Some(&Value::string("Changing state"))
    );
    assert_eq!(history[1].get("role").and_then(Value::as_str), Some("user"));
    assert!(request.encode().find("jecode_request_state").is_none());
}

#[test]
fn mid_system_state_preserves_the_cached_prefix_as_history_grows() {
    let system = Value::object([
        ("role", Value::string("system")),
        ("content", Value::string("Stable instructions")),
    ]);
    let ledger = Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Stable ledger")),
    ]);
    let state = Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Changing runtime state")),
        ("jecode_request_state", Value::Bool(true)),
    ]);
    let cold = wire::request(
        "claude-opus-5-5",
        Effort::Medium,
        &[system.clone(), ledger.clone(), state.clone()],
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let growing = wire::request(
        "claude-opus-5-5",
        Effort::Medium,
        &[
            system,
            ledger,
            Value::object([
                ("role", Value::string("assistant")),
                ("content", Value::string("Earlier answer")),
            ]),
            Value::object([
                ("role", Value::string("user")),
                ("content", Value::string("Follow-up")),
            ]),
            state,
        ],
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let first = cold.get("messages").and_then(Value::as_array).unwrap();
    let next = growing.get("messages").and_then(Value::as_array).unwrap();
    assert_eq!(cold.get("system"), growing.get("system"));
    assert_eq!(first[0], next[0]);
    assert!(
        first[0].get("content").and_then(Value::as_array).unwrap()[0]
            .get("cache_control")
            .is_some()
    );
    assert_eq!(first[1].get("role").and_then(Value::as_str), Some("system"));
    assert_eq!(
        next.last().unwrap().get("role").and_then(Value::as_str),
        Some("system")
    );
    assert!(cold.get("cache_control").is_none());
}

#[test]
fn mid_system_is_limited_to_documented_model_ids() {
    let state = Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Runtime state")),
        ("jecode_request_state", Value::Bool(true)),
    ]);
    for (model, role) in [
        ("claude-opus-5-5-20261031", "system"),
        ("claude-sonnet-5", "user"),
        ("unknown-model", "user"),
    ] {
        let request = wire::request(
            model,
            Effort::Default,
            std::slice::from_ref(&state),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let message = &request.get("messages").and_then(Value::as_array).unwrap()[0];
        assert_eq!(message.get("role").and_then(Value::as_str), Some(role));
    }
}
