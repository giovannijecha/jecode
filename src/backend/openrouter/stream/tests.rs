use super::*;

#[test]
fn the_final_accounting_choice_can_repeat_the_terminal_finish_reason() {
    let mut stream = Stream::default();
    let mut reported = Vec::new();
    let content = delta(
        Value::object([("content", Value::string("Done"))]),
        Value::string("stop"),
    );
    let accounting = "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":20,\"total_tokens\":120}}\n\ndata: [DONE]\n\n";
    stream
        .feed(format!("{content}{accounting}").as_bytes(), &mut |update| {
            if let Update::Usage(usage) = update {
                reported.push(usage.context_tokens());
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(reported, [Some(120)]);
    let completion = stream.finish("").unwrap();
    assert_eq!(completion.text, "Done");
    assert_eq!(completion.usage.unwrap().context_tokens(), Some(120));
}
use std::cell::RefCell;

fn delta(value: Value, finish: Value) -> String {
    format!(
        "data: {}\n\n",
        Value::object([(
            "choices",
            Value::Array(vec![Value::object([
                ("index", Value::number(0)),
                ("delta", value),
                ("finish_reason", finish),
            ])])
        )])
        .encode()
    )
}

#[test]
fn fragmented_utf8_sse_preserves_reasoning_and_assembles_tool_arguments() {
    let reasoning = Value::object([
        ("type", Value::string("reasoning.encrypted")),
        ("data", Value::string("opaque-original")),
    ]);
    let mut wire = String::from(": OPENROUTER PROCESSING\n\n");
    wire.push_str(&delta(
        Value::object([("reasoning_details", Value::Array(vec![reasoning.clone()]))]),
        Value::Null,
    ));
    wire.push_str(&delta(
        Value::object([("content", Value::string("Città 🙂"))]),
        Value::Null,
    ));
    for (id, args) in [(Some("call-1"), "{\"path\":"), (None, "\"file.rs\"}")] {
        let mut call = Value::object([
            ("index", Value::number(0)),
            (
                "function",
                Value::object([
                    (
                        "name",
                        Value::string(if id.is_some() { "read" } else { "" }),
                    ),
                    ("arguments", Value::string(args)),
                ]),
            ),
        ]);
        if let Some(id) = id
            && let Value::Object(fields) = &mut call
        {
            fields.insert("id".into(), Value::string(id));
        }
        wire.push_str(&delta(
            Value::object([("tool_calls", Value::Array(vec![call]))]),
            Value::Null,
        ));
    }
    wire.push_str(&delta(Value::object([]), Value::string("tool_calls")));
    wire.push_str("data: {\"choices\":[],\"usage\":{}}\n\ndata: [DONE]\n\n");
    let mut stream = Stream::default();
    let mut text = String::new();
    let mut thinking = false;
    for chunk in wire.as_bytes().chunks(3) {
        stream
            .feed(chunk, &mut |update| {
                match update {
                    Update::TextDelta(value) => text.push_str(&value),
                    Update::Reasoning => thinking = true,
                    Update::Working => {}
                    Update::Usage(_) => {}
                    Update::Waiting { .. } => unreachable!("parser cannot dispatch a request"),
                };
                Ok(())
            })
            .unwrap();
    }
    let completion = stream.finish(&wire).unwrap();
    assert!(thinking);
    assert_eq!(text, "Città 🙂");
    assert_eq!(completion.calls[0].arguments, "{\"path\":\"file.rs\"}");
    assert_eq!(
        completion
            .message
            .get("reasoning_details")
            .unwrap()
            .as_array()
            .unwrap(),
        &[reasoning]
    );
}

#[test]
fn usage_only_chunks_emit_live_counts_and_survive_completion() {
    let mut stream = Stream::default();
    let reported = RefCell::new(Vec::new());
    let mut emit = |update| {
        if let Update::Usage(usage) = update {
            reported.borrow_mut().push(usage);
        }
        Ok(())
    };
    let text = delta(
        Value::object([("content", Value::string("Done"))]),
        Value::string("stop"),
    );
    stream.feed(text.as_bytes(), &mut emit).unwrap();
    let input = "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":10,\"total_tokens\":110}}\n\n";
    stream.feed(input.as_bytes(), &mut emit).unwrap();
    assert_eq!(
        reported
            .borrow()
            .iter()
            .map(|usage| usage.context_tokens())
            .collect::<Vec<_>>(),
        [Some(110)]
    );
    let partial = "data: {\"choices\":[],\"usage\":{\"completion_tokens\":20}}\n\n";
    stream.feed(partial.as_bytes(), &mut emit).unwrap();
    assert_eq!(
        reported.borrow().last().unwrap().context_tokens(),
        Some(120)
    );
    let total = "data: {\"choices\":[],\"usage\":{\"total_tokens\":125}}\n\ndata: [DONE]\n\n";
    stream.feed(total.as_bytes(), &mut emit).unwrap();
    assert_eq!(
        reported
            .borrow()
            .iter()
            .map(|usage| usage.context_tokens())
            .collect::<Vec<_>>(),
        [Some(110), Some(120), Some(125)]
    );
    let completion = stream.finish("").unwrap();
    let usage = completion.usage.unwrap();
    assert_eq!(usage.input_tokens, Some(100));
    assert_eq!(usage.output_tokens, Some(20));
    assert_eq!(usage.total_tokens, Some(125));
}

#[test]
fn error_chunk_still_emits_provider_usage() {
    let mut stream = Stream::default();
    let mut reported = Vec::new();
    let failure = "data: {\"usage\":{\"total_tokens\":0},\"error\":{\"message\":\"failed\"}}\n\n";
    assert!(
        stream
            .feed(failure.as_bytes(), &mut |update| {
                if let Update::Usage(usage) = update {
                    reported.push(usage.context_tokens());
                }
                Ok(())
            })
            .is_err()
    );
    assert_eq!(reported, [Some(0)]);
}

#[test]
fn incomplete_or_failed_streams_never_return_executable_calls() {
    let wire = delta(
        Value::object([("content", Value::string("partial"))]),
        Value::Null,
    );
    let mut stream = Stream::default();
    stream.feed(wire.as_bytes(), &mut |_| Ok(())).unwrap();
    assert!(
        stream
            .finish(&wire)
            .err()
            .unwrap()
            .contains("before [DONE]")
    );
    let mut stream = Stream::default();
    assert!(
        stream
            .feed(
                b"data: {\"error\":{\"message\":\"failed\"}}\n\n",
                &mut |_| Ok(())
            )
            .is_err()
    );
    let wire = delta(Value::object([]), Value::string("length")) + "data: [DONE]\n\n";
    let mut stream = Stream::default();
    stream.feed(wire.as_bytes(), &mut |_| Ok(())).unwrap();
    assert!(stream.finish(&wire).is_err());
}

#[test]
fn null_delta_fields_are_optional_and_missing_tool_indexes_are_rejected() {
    let wire = delta(
        Value::object([
            ("content", Value::string("text")),
            ("tool_calls", Value::Null),
        ]),
        Value::string("stop"),
    ) + "data: [DONE]\n\n";
    let mut stream = Stream::default();
    stream.feed(wire.as_bytes(), &mut |_| Ok(())).unwrap();
    assert_eq!(stream.finish(&wire).unwrap().text, "text");
    let call = Value::object([
        ("index", Value::number(1)),
        ("id", Value::string("call-1")),
        (
            "function",
            Value::object([
                ("name", Value::string("read")),
                ("arguments", Value::string("{}")),
            ]),
        ),
    ]);
    let wire = delta(
        Value::object([("tool_calls", Value::Array(vec![call]))]),
        Value::string("tool_calls"),
    ) + "data: [DONE]\n\n";
    let mut stream = Stream::default();
    stream.feed(wire.as_bytes(), &mut |_| Ok(())).unwrap();
    assert!(
        stream
            .finish(&wire)
            .err()
            .unwrap()
            .contains("missing tool call indexes")
    );
}

#[test]
fn the_serving_provider_and_reported_usage_survive_streaming() {
    let mut wire = String::from(
        "data: {\"provider\":\"Baidu\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
    );
    wire.push_str(&delta(Value::object([]), Value::string("stop")));
    wire.push_str(
        "data: {\"provider\":\"Baidu\",\"choices\":[],\"usage\":{\"cost\":0.0004,\"prompt_tokens_details\":{\"cached_tokens\":64,\"cache_write_tokens\":128}}}\n\ndata: [DONE]\n\n",
    );
    let mut stream = Stream::default();
    stream.feed(wire.as_bytes(), &mut |_| Ok(())).unwrap();
    let completion = stream.finish(&wire).unwrap();
    assert_eq!(
        completion
            .charges
            .as_ref()
            .and_then(|charges| charges.providers.first())
            .map(String::as_str),
        Some("Baidu")
    );
    let charges = completion.charges.as_ref().unwrap();
    assert_eq!(completion.usage.unwrap().cache_write_tokens, Some(128));
    assert_eq!(charges.cache_written, 128);
    assert_eq!(
        charges.describe().unwrap(),
        "Served by Baidu · $0.0004 charged · 64 cached · 128 cache write tokens"
    );
    let plain = parse_completion(
        crate::json::parse(
            r#"{"provider":"DeepInfra","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"ok"}}]}"#,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        plain
            .charges
            .as_ref()
            .and_then(|charges| charges.providers.first())
            .map(String::as_str),
        Some("DeepInfra")
    );
}

#[test]
fn routing_metadata_reports_only_the_selected_provider_for_json_and_streams() {
    let metadata = r#"{"endpoints":{"available":[{"provider":"Unused","selected":false},{"provider":"Baidu","selected":true}]}}"#;
    let response = format!(
        r#"{{"choices":[{{"finish_reason":"stop","message":{{"role":"assistant","content":"ok"}}}}],"openrouter_metadata":{metadata}}}"#
    );
    let parsed = parse_completion(crate::json::parse(&response).unwrap()).unwrap();
    assert_eq!(
        parsed
            .charges
            .as_ref()
            .and_then(|charges| charges.providers.first())
            .map(String::as_str),
        Some("Baidu")
    );
    let wire = delta(
        Value::object([("content", Value::string("ok"))]),
        Value::string("stop"),
    ) + "data: {\"choices\":[],\"usage\":{\"cost\":0.002}}\n\n"
        + &format!(
            "data: {{\"choices\":[],\"usage\":null,\"openrouter_metadata\":{metadata}}}\n\ndata: [DONE]\n\n"
        );
    let mut stream = Stream::default();
    stream.feed(wire.as_bytes(), &mut |_| Ok(())).unwrap();
    let completion = stream.finish(&wire).unwrap();
    assert_eq!(
        completion
            .charges
            .as_ref()
            .and_then(|charges| charges.providers.first())
            .map(String::as_str),
        Some("Baidu")
    );
    assert_eq!(
        completion.charges.as_ref().map(|charges| charges.cost),
        Some(0.002)
    );
}

#[test]
#[ignore = "local release benchmark; no provider request"]
fn tool_argument_stream_bench() {
    use std::time::Instant;

    for fragments in [512, 2048, 4096] {
        let mut wire = String::new();
        for index in 0..fragments {
            let call = Value::object([
                ("index", Value::number(0)),
                ("id", Value::string("call-1")),
                (
                    "function",
                    Value::object([
                        ("name", Value::string(if index == 0 { "read" } else { "" })),
                        ("arguments", Value::string("x".repeat(64))),
                    ]),
                ),
            ]);
            wire.push_str(&delta(
                Value::object([("tool_calls", Value::Array(vec![call]))]),
                Value::Null,
            ));
        }
        wire.push_str(&delta(Value::object([]), Value::string("tool_calls")));
        wire.push_str("data: [DONE]\n\n");
        let mut timings = Vec::new();
        for sample in 0..7 {
            let mut stream = Stream::default();
            let started = Instant::now();
            stream.feed(wire.as_bytes(), &mut |_| Ok(())).unwrap();
            let completion = stream.finish(&wire).unwrap();
            let elapsed = started.elapsed();
            assert_eq!(completion.calls[0].arguments.len(), fragments * 64);
            if sample >= 2 {
                timings.push(elapsed.as_millis());
            }
        }
        eprintln!("tool argument fragments={fragments}, 64 bytes each, warmup=2, ms={timings:?}");
    }
}
