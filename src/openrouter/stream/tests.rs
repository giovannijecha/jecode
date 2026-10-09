use super::*;

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
                    Update::Text(value) => text = value,
                    Update::Reasoning => thinking = true,
                    Update::Working | Update::Retry { .. } | Update::RetryFinished => {}
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
