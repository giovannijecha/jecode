use super::*;
use std::cell::RefCell;

#[test]
fn usage_arrives_before_completion_and_preserves_provider_total() {
    let mut stream = Stream::default();
    let reported = RefCell::new(Vec::new());
    let mut emit = |update| {
        if let Update::Usage(usage) = update {
            reported.borrow_mut().push(usage);
        }
        Ok(())
    };
    let update = Value::object([
        ("type", Value::string("response.in_progress")),
        (
            "response",
            Value::object([(
                "usage",
                Value::object([
                    ("input_tokens", Value::number(500)),
                    ("output_tokens", Value::number(0)),
                    ("total_tokens", Value::number(500)),
                ]),
            )]),
        ),
    ]);
    stream.event(&update.encode(), &mut emit).unwrap();
    assert_eq!(
        reported
            .borrow()
            .iter()
            .map(|usage| usage.context_tokens())
            .collect::<Vec<_>>(),
        [Some(500)]
    );

    let partial = Value::object([
        ("type", Value::string("response.in_progress")),
        (
            "response",
            Value::object([(
                "usage",
                Value::object([("output_tokens", Value::number(20))]),
            )]),
        ),
    ]);
    stream.event(&partial.encode(), &mut emit).unwrap();
    assert_eq!(
        reported.borrow().last().unwrap().context_tokens(),
        Some(520)
    );

    let completed = Value::object([
        ("type", Value::string("response.completed")),
        (
            "response",
            Value::object([
                ("status", Value::string("completed")),
                (
                    "output",
                    Value::Array(vec![Value::object([
                        ("type", Value::string("message")),
                        ("role", Value::string("assistant")),
                        (
                            "content",
                            Value::Array(vec![Value::object([
                                ("type", Value::string("output_text")),
                                ("text", Value::string("Done")),
                            ])]),
                        ),
                    ])]),
                ),
                (
                    "usage",
                    Value::object([
                        ("input_tokens", Value::number(500)),
                        ("output_tokens", Value::number(20)),
                        ("total_tokens", Value::number(530)),
                    ]),
                ),
            ]),
        ),
    ]);
    stream.event(&completed.encode(), &mut emit).unwrap();
    assert_eq!(
        reported
            .borrow()
            .iter()
            .map(|usage| usage.context_tokens())
            .collect::<Vec<_>>(),
        [Some(500), Some(520), Some(530)]
    );
    assert_eq!(
        stream.finish().unwrap().usage.unwrap().total_tokens,
        Some(530)
    );
}

#[test]
fn failed_and_incomplete_responses_emit_reported_usage() {
    for (kind, total) in [("response.failed", 0), ("response.incomplete", 7)] {
        let mut stream = Stream::default();
        let mut reported = Vec::new();
        let event = Value::object([
            ("type", Value::string(kind)),
            (
                "response",
                Value::object([
                    (
                        "usage",
                        Value::object([("total_tokens", Value::number(total))]),
                    ),
                    (
                        "error",
                        Value::object([("message", Value::string("Stopped"))]),
                    ),
                ]),
            ),
        ]);
        stream
            .event(&event.encode(), &mut |update| {
                if let Update::Usage(usage) = update {
                    reported.push(usage.context_tokens());
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(reported, [Some(total)]);
        assert!(stream.finish().is_err());
    }
}

#[test]
fn assistant_messages_are_separated_in_stream_and_saved_completion() {
    let first = Value::object([
        ("type", Value::string("message")),
        ("role", Value::string("assistant")),
        ("phase", Value::string("commentary")),
        (
            "content",
            Value::Array(vec![Value::object([
                ("type", Value::string("output_text")),
                ("text", Value::string("Checking the file.")),
            ])]),
        ),
    ]);
    let mut second = first.clone();
    if let Value::Object(fields) = &mut second {
        fields.insert("phase".into(), Value::string("final_answer"));
        fields.insert(
            "content".into(),
            Value::Array(vec![Value::object([
                ("type", Value::string("output_text")),
                ("text", Value::string("All good.")),
            ])]),
        );
    }
    for with_open_events in [true, false] {
        let mut stream = Stream::default();
        let mut observed = String::new();
        let mut emit = |update| {
            if let Update::TextDelta(delta) = update {
                observed.push_str(&delta);
            }
            Ok(())
        };
        for (index, item, deltas) in [
            (0, &first, vec!["Checking", " the file."]),
            (2, &second, vec!["", "All good."]),
        ] {
            if with_open_events {
                stream
                    .event(
                        &Value::object([
                            ("type", Value::string("response.output_item.added")),
                            ("output_index", Value::number(index)),
                            ("item", item.clone()),
                        ])
                        .encode(),
                        &mut emit,
                    )
                    .unwrap();
            }
            for delta in deltas {
                stream
                    .event(
                        &Value::object([
                            ("type", Value::string("response.output_text.delta")),
                            ("output_index", Value::number(index)),
                            ("delta", Value::string(delta)),
                        ])
                        .encode(),
                        &mut emit,
                    )
                    .unwrap();
            }
        }
        stream
            .event(
                &Value::object([
                    ("type", Value::string("response.completed")),
                    (
                        "response",
                        Value::object([
                            ("status", Value::string("completed")),
                            ("output", Value::Array(vec![first.clone(), second.clone()])),
                        ]),
                    ),
                ])
                .encode(),
                &mut emit,
            )
            .unwrap();
        let completion = stream.finish().unwrap();
        assert_eq!(completion.text, "Checking the file.\n\nAll good.");
        assert_eq!(observed, completion.text);
        assert_eq!(
            completion.message.get("content").and_then(Value::as_str),
            Some(observed.as_str())
        );
    }
}

#[test]
fn top_level_stream_error_preserves_retry_classification_and_details() {
    let mut stream = Stream::default();
    stream.feed(b"data: {\"type\":\"error\",\"code\":\"server_is_overloaded\",\"message\":\"Please retry\",\"param\":\"model\",\"sequence_number\":1}\n\n", &mut |_| Ok(())).unwrap();
    let failure = stream.finish().err().unwrap();
    assert_eq!(failure.kind, FailureKind::Temporary);
    assert!(failure.message.contains("server_is_overloaded"));
    assert!(failure.message.contains("Please retry"));
    assert!(failure.message.contains("param: model"));
}

#[test]
fn cached_input_is_reported_with_the_completion() {
    let bytes = concat!(
        "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Done\"}]}}

",
        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":9000,\"input_tokens_details\":{\"cached_tokens\":8192},\"output_tokens\":5}}}

"
    );
    let mut stream = Stream::default();
    stream.feed(bytes.as_bytes(), &mut |_| Ok(())).unwrap();
    let charges = stream.finish().unwrap().charges.unwrap();
    assert_eq!((charges.requests, charges.cached), (1, 8192));
    assert_eq!(charges.describe().as_deref(), Some("8.2K cached"));
}

#[test]
fn opened_reasoning_and_call_items_report_progress_without_summaries() {
    let bytes = concat!(
        "data: {\"type\":\"response.created\",\"response\":{}}

",
        "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"reasoning\"}}

",
        "data: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"type\":\"message\"}}

",
        "data: {\"type\":\"response.output_item.added\",\"output_index\":2,\"item\":{\"type\":\"function_call\"}}

",
    );
    let mut seen = Vec::new();
    Stream::default()
        .feed(bytes.as_bytes(), &mut |update| {
            seen.push(match update {
                Update::Reasoning => "reasoning",
                Update::Working => "working",
                _ => "other",
            });
            Ok(())
        })
        .unwrap();
    assert_eq!(seen, ["reasoning", "working"]);
}

#[test]
fn completed_empty_output_keeps_items_already_delivered_by_the_chatgpt_route() {
    let bytes = concat!(
        "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"reasoning\",\"encrypted_content\":\"opaque\"}}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"commentary\",\"content\":[{\"type\":\"output_text\",\"text\":\"Checking the file\"}]}}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"output_index\":2,\"item\":{\"type\":\"function_call\",\"call_id\":\"read-1\",\"namespace\":\"jecode\",\"name\":\"read\",\"arguments\":\"{\\\"path\\\":\\\"probe.txt\\\"}\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":12,\"output_tokens\":5}}}\n\n"
    );
    let mut stream = Stream::default();
    for chunk in bytes.as_bytes().chunks(11) {
        stream.feed(chunk, &mut |_| Ok(())).unwrap();
    }
    let completion = stream.finish().unwrap();
    assert_eq!(completion.text, "Checking the file");
    assert_eq!(completion.calls[0].name, "read");
    assert_eq!(
        completion
            .message
            .get("openai_items")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(completion.usage.unwrap().input_tokens, Some(12));
    assert!(completion.charges.is_none());
}

#[test]
fn fragmented_stream_waits_for_completion_and_preserves_native_order() {
    let mut stream = Stream::default();
    let bytes = concat!(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"reasoning\",\"encrypted_content\":\"secret\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[{\"type\":\"reasoning\"},{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"final\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hi\"}]}],\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}}\n\n"
    );
    let mut seen = Vec::new();
    for chunk in bytes.as_bytes().chunks(7) {
        stream
            .feed(chunk, &mut |update| {
                if let Update::TextDelta(text) = update {
                    seen.push(text);
                }
                Ok(())
            })
            .unwrap();
    }
    let result = stream.finish().unwrap();
    assert_eq!(seen, ["Hi"]);
    assert_eq!(result.text, "Hi");
    let items = result
        .message
        .get("openai_items")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        items[0].get("encrypted_content").and_then(Value::as_str),
        Some("secret")
    );
    assert_eq!(result.usage.unwrap().input_tokens, Some(3));
}

#[test]
fn interrupted_or_failed_stream_never_returns_calls() {
    let call = "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"call_id\":\"c\",\"name\":\"write\",\"arguments\":\"{}\"}}\n\n";
    let mut stream = Stream::default();
    stream.feed(call.as_bytes(), &mut |_| Ok(())).unwrap();
    assert_eq!(stream.finish().err().unwrap().kind, FailureKind::Temporary);
    let mut stream = Stream::default();
    stream.feed(call.as_bytes(), &mut |_| Ok(())).unwrap();
    stream.feed(b"data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\",\"message\":\"limit\"}}}\n\n", &mut |_| Ok(())).unwrap();
    let failure = stream.finish().err().unwrap();
    assert_eq!(failure.kind, FailureKind::Terminal);
    assert!(failure.diagnostic.as_deref().unwrap().contains("\"error\""));
    assert!(
        failure
            .diagnostic
            .as_deref()
            .unwrap()
            .contains("response.failed")
    );
}

#[test]
fn completed_refusal_is_displayed_as_assistant_text() {
    let mut stream = Stream::default();
    let bytes = concat!(
        "data: {\"type\":\"response.refusal.delta\",\"delta\":\"I cannot help\"}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"refusal\",\"refusal\":\"I cannot help\"}]}]}}\n\n",
    );
    let mut deltas = Vec::new();
    stream
        .feed(bytes.as_bytes(), &mut |update| {
            if let Update::TextDelta(text) = update {
                deltas.push(text);
            }
            Ok(())
        })
        .unwrap();
    let completion = stream.finish().unwrap();
    assert_eq!(deltas, ["I cannot help"]);
    assert_eq!(completion.text, "I cannot help");
    assert!(completion.calls.is_empty());
}
