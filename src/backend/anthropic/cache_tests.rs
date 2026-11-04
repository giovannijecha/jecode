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
    // The automatic breakpoint covers the newly sent state at the end.
    assert!(request.get("cache_control").is_some());
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
fn newly_sent_state_is_covered_by_the_automatic_breakpoint() {
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
    assert!(request.get("cache_control").is_some());
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
    assert!(cold.get("cache_control").is_some());
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

#[test]
fn shared_instructions_are_cached_apart_from_the_folder_part() {
    let shared = "Shared agent prompt.";
    let content = format!("{shared}\n\nEnvironment:\n{{\"working_directory\":\"/a\"}}");
    let mut messages = vec![Value::object([
        ("role", Value::string("system")),
        ("content", Value::string(&content)),
        ("jecode_cache_prefix", Value::number(shared.len())),
    ])];
    for index in 0..6 {
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
    let system = request.get("system").and_then(Value::as_array).unwrap();
    let texts: Vec<_> = system
        .iter()
        .map(|block| block.get("text").and_then(Value::as_str).unwrap())
        .collect();
    assert_eq!(
        texts[texts.len() - 2..],
        [shared, "Environment:\n{\"working_directory\":\"/a\"}"]
    );
    let shared_block = &system[system.len() - 2];
    assert_eq!(
        shared_block.get("cache_control"),
        Some(&Value::object([
            ("type", Value::string("ephemeral")),
            ("ttl", Value::string("1h")),
        ]))
    );
    assert!(system.last().unwrap().get("cache_control").is_none());
    let encoded = request.encode();
    assert!(
        encoded.matches("\"cache_control\"").count() <= 4,
        "{encoded}"
    );
    assert!(!encoded.contains("jecode_cache_prefix"));
}

#[test]
fn one_hour_retention_extends_every_breakpoint() {
    let shared = "Shared agent prompt.";
    let content = format!("{shared}\n\nEnvironment:\n{{}}");
    let short = Value::object([("type", Value::string("ephemeral"))]);
    let long = Value::object([
        ("type", Value::string("ephemeral")),
        ("ttl", Value::string("1h")),
    ]);
    for (retention, expected) in [(None, short), (Some("1h"), long.clone())] {
        let mut system = Value::object([
            ("role", Value::string("system")),
            ("content", Value::string(&content)),
            ("jecode_cache_prefix", Value::number(shared.len())),
        ]);
        if let (Some(retention), Value::Object(fields)) = (retention, &mut system) {
            fields.insert("jecode_cache_retention".into(), Value::string(retention));
        }
        let mut messages = vec![system];
        for index in 0..6 {
            let role = if index % 2 == 0 { "user" } else { "assistant" };
            messages.push(Value::object([
                ("role", Value::string(role)),
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
        // The shared block is always long-lived; it precedes every other
        // breakpoint in the prompt, as mixed lifetimes require.
        let mut system = Vec::new();
        collect_controls(request.get("system").unwrap(), &mut system);
        assert_eq!(system, std::slice::from_ref(&long));
        let mut rest = vec![request.get("cache_control").unwrap().clone()];
        collect_controls(request.get("messages").unwrap(), &mut rest);
        assert_eq!(rest.len(), 3, "{rest:?}");
        assert!(rest.iter().all(|control| *control == expected), "{rest:?}");
        assert!(!request.encode().contains("jecode_cache_retention"));
    }
}

fn collect_controls(value: &Value, found: &mut Vec<Value>) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if key == "cache_control" {
                    found.push(value.clone());
                } else {
                    collect_controls(value, found);
                }
            }
        }
        Value::Array(values) => values
            .iter()
            .for_each(|value| collect_controls(value, found)),
        _ => {}
    }
}
