use jecode::{
    json::{self, Value},
    providers::openai_account::{Error, Limits, Progress, ResponseStream, Status},
};
use std::ops::ControlFlow;

fn frame(data: &str) -> String {
    format!("data: {data}\n\n")
}
fn done(index: usize, item: &str) -> String {
    frame(&format!(
        r#"{{"type":"response.output_item.done","output_index":{index},"item":{item}}}"#
    ))
}
fn delta(output: usize, content: usize, item_id: &str, text: &str, sequence: usize) -> String {
    frame(&format!(
        r#"{{"type":"response.output_text.delta","output_index":{output},"content_index":{content},"item_id":"{item_id}","delta":"{text}","sequence_number":{sequence}}}"#
    ))
}
fn message(id: &str, content: &str) -> String {
    format!(r#"{{"type":"message","id":"{id}","status":"completed","content":{content}}}"#)
}
fn terminal(status: &str, output: &str, extra: &str) -> String {
    frame(&format!(
        r#"{{"type":"response.{status}","response":{{"id":"resp_test","status":"{status}","output":{output}{extra}}}}}"#
    ))
}
const MESSAGE: &str = r#"{"type":"message","id":"msg_1","status":"completed","content":[{"type":"output_text","text":"Hi 🚀"}]}"#;
const TOOL: &str = r#"{"type":"function_call","id":"fc_1","status":"completed","call_id":"call_1","name":"read_file","arguments":"{\"path\":\"README.md\"}"}"#;
fn consume(text: &str) -> Result<jecode::providers::openai_account::Response, Error> {
    let mut stream = ResponseStream::default();
    stream.push(text.as_bytes(), |_| ControlFlow::Continue(()))?;
    stream.finish()
}

#[test]
fn fragmented_stream_retains_opaque_items_usage_and_empty_terminal_fallback() {
    let reasoning =
        r#"{"type":"reasoning","id":"rs_1","encrypted_content":"synthetic-opaque","summary":[]}"#;
    let wire = frame(r#"{"type":"response.output_text.delta","delta":"Hi 🚀"}"#)
        + &done(0, MESSAGE)
        + &done(1, reasoning)
        + &done(2, TOOL)
        + &terminal(
            "completed",
            "[]",
            r#", "usage":{"input_tokens":120,"output_tokens":30,"input_tokens_details":{"cached_tokens":100},"output_tokens_details":{"reasoning_tokens":20}}"#,
        );
    for split in 0..=wire.len() {
        let mut stream = ResponseStream::default();
        let mut text = String::new();
        let mut callback = |event: Progress<'_>| {
            if let Progress::Text(delta) = event {
                text.push_str(delta);
            }
            ControlFlow::Continue(())
        };
        stream
            .push(&wire.as_bytes()[..split], &mut callback)
            .unwrap();
        if !stream.is_finished() {
            stream
                .push(&wire.as_bytes()[split..], &mut callback)
                .unwrap();
        }
        let result = stream.finish().unwrap();
        assert_eq!(text, "Hi 🚀");
        assert_eq!(result.text, text);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(
            result.tool_calls[0].arguments.get("path").unwrap().text(),
            Some("README.md")
        );
        assert_eq!(
            result.output[1].get("encrypted_content").unwrap().text(),
            Some("synthetic-opaque")
        );
        assert_eq!(result.usage.cached, Some(100));
        assert_eq!(result.usage.reasoning, Some(20));
    }
    let mut stream = ResponseStream::default();
    for byte in wire.bytes() {
        stream.push(&[byte], |_| ControlFlow::Continue(())).unwrap();
    }
    assert_eq!(stream.finish().unwrap().status, Status::Completed);
}

#[test]
fn tools_require_terminal_success_and_valid_complete_arguments() {
    assert_eq!(consume(&done(0, TOOL)), Err(Error::MissingTerminal));
    assert_eq!(
        consume(&(done(0, TOOL) + &frame("[DONE]"))),
        Err(Error::MissingTerminal)
    );
    let incomplete = consume(&terminal("incomplete", &format!("[{TOOL}]"), "")).unwrap();
    assert_eq!(incomplete.status, Status::Incomplete);
    assert!(incomplete.tool_calls.is_empty());
    let refusal = r#"{"type":"message","content":[{"type":"refusal","refusal":"Cannot do that"}]}"#;
    let refused = consume(&terminal("completed", &format!("[{TOOL},{refusal}]"), "")).unwrap();
    assert_eq!(refused.status, Status::Refused);
    assert!(refused.tool_calls.is_empty());
    for tool in [
        TOOL.replace("completed", "in_progress"),
        TOOL.replace("read_file", ""),
        TOOL.replace(r#"{\"path\":\"README.md\"}"#, "[]"),
    ] {
        assert!(consume(&terminal("completed", &format!("[{tool}]"), "")).is_err());
    }
    assert_eq!(
        consume(&terminal(
            "completed",
            &format!("[{TOOL},{}]", TOOL.replace("fc_1", "fc_2")),
            ""
        )),
        Err(Error::InvalidTool)
    );
    assert_eq!(
        consume(&terminal(
            "completed",
            &format!("[{MESSAGE},{MESSAGE}]"),
            ""
        )),
        Err(Error::ConflictingOutput)
    );
}

#[test]
fn rejects_conflicts_and_preserves_unknown_usage() {
    assert_eq!(
        consume(&(done(0, MESSAGE) + &done(0, MESSAGE))),
        Err(Error::ConflictingOutput)
    );
    assert_eq!(
        consume(&(done(1, MESSAGE) + &terminal("completed", "[]", ""))),
        Err(Error::ConflictingOutput)
    );
    assert_eq!(
        consume(&(done(0, MESSAGE) + &terminal("completed", &format!("[{TOOL}]"), ""))),
        Err(Error::ConflictingOutput)
    );
    let result = consume(&terminal("completed", &format!("[{MESSAGE}]"), "")).unwrap();
    assert_eq!(result.usage.input, None);
    for usage in [
        r#"{"input_tokens":-1}"#,
        r#"{"output_tokens":1.0}"#,
        r#"{"input_tokens":5,"input_tokens_details":{"cached_tokens":6}}"#,
        r#"{"output_tokens_details":false}"#,
    ] {
        assert_eq!(
            consume(&terminal("completed", "[]", &format!(",\"usage\":{usage}"))),
            Err(Error::InvalidUsage)
        );
    }
}

#[test]
fn cancellation_stops_current_chunk_and_cannot_yield_tools() {
    let wire = frame(r#"{"type":"response.output_text.delta","delta":"first"}"#)
        + &terminal("completed", &format!("[{TOOL}]"), "");
    let mut stream = ResponseStream::default();
    let mut callbacks = 0;
    assert_eq!(
        stream.push(wire.as_bytes(), |_| {
            callbacks += 1;
            ControlFlow::Break(())
        }),
        Err(Error::Cancelled)
    );
    assert_eq!(callbacks, 1);
    assert_eq!(
        stream.push(b"", |_| ControlFlow::Continue(())),
        Err(Error::Closed)
    );
    assert_eq!(stream.finish(), Err(Error::Cancelled));
    let mut stream = ResponseStream::default();
    stream.cancel();
    assert_eq!(stream.finish(), Err(Error::Cancelled));
}

#[test]
fn failure_limits_and_truncation_never_become_success() {
    let error = consume(&frame(r#"{"type":"error","message":"synthetic secret"}"#)).unwrap_err();
    assert!(matches!(error, Error::RemoteFailure(_)));
    assert!(!error.to_string().contains("synthetic secret"));
    for wire in [
        frame("{"),
        frame(r#"{"type":"x","type":"response.completed"}"#),
        terminal("completed", "[]", "").trim_end().to_owned(),
    ] {
        assert!(consume(&wire).is_err());
    }
    for limits in [
        Limits {
            event_bytes: 5,
            ..Default::default()
        },
        Limits {
            output_bytes: 1,
            ..Default::default()
        },
        Limits {
            wire_bytes: 1,
            ..Default::default()
        },
    ] {
        let mut stream = ResponseStream::new(limits);
        assert!(
            stream
                .push(
                    terminal("completed", &format!("[{MESSAGE}]"), "").as_bytes(),
                    |_| ControlFlow::Continue(())
                )
                .is_err()
        );
        assert!(stream.finish().is_err());
    }
}

#[test]
fn terminal_must_match_progress_and_its_own_status() {
    let created = frame(r#"{"type":"response.created","response":{"id":"another_response"}}"#);
    assert_eq!(
        consume(&(created + &terminal("completed", "[]", ""))),
        Err(Error::ConflictingOutput)
    );
    let prefix = frame(r#"{"type":"response.output_text.delta","delta":"different"}"#);
    assert_eq!(
        consume(&(prefix + &terminal("completed", &format!("[{MESSAGE}]"), ""))),
        Err(Error::ConflictingOutput)
    );
    let invalid = terminal("completed", "[]", "")
        .replace(r#""status":"completed""#, r#""status":"incomplete""#);
    assert_eq!(consume(&invalid), Err(Error::InvalidEvent));
    let value = json::parse(MESSAGE, Default::default()).unwrap();
    assert!(matches!(value, Value::Object(_)));
}

#[test]
fn aggregate_budgets_cover_many_individually_valid_events() {
    let delta = frame(r#"{"type":"response.output_text.delta","delta":"ab"}"#);
    let mut stream = ResponseStream::new(Limits {
        output_bytes: 3,
        ..Default::default()
    });
    stream
        .push(delta.as_bytes(), |_| ControlFlow::Continue(()))
        .unwrap();
    assert_eq!(
        stream.push(delta.as_bytes(), |_| ControlFlow::Continue(())),
        Err(Error::Limit)
    );
    assert_eq!(stream.finish(), Err(Error::Limit));
    let mut stream = ResponseStream::new(Limits {
        items: 1,
        ..Default::default()
    });
    stream
        .push(done(0, MESSAGE).as_bytes(), |_| ControlFlow::Continue(()))
        .unwrap();
    assert!(
        stream
            .push(done(1, TOOL).as_bytes(), |_| ControlFlow::Continue(()))
            .is_err()
    );
    let mut stream = ResponseStream::new(Limits {
        wire_bytes: delta.len(),
        ..Default::default()
    });
    stream
        .push(delta.as_bytes(), |_| ControlFlow::Continue(()))
        .unwrap();
    assert_eq!(
        stream.push(b"\n", |_| ControlFlow::Continue(())),
        Err(Error::Limit)
    );
}

#[test]
fn successful_terminal_is_final_and_reasoning_is_separate_progress() {
    let mut stream = ResponseStream::default();
    let mut reasoning = String::new();
    let wire = frame(r#"{"type":"response.reasoning_summary_text.delta","delta":"Checking"}"#)
        + &done(0, MESSAGE)
        + &terminal("completed", &format!("[{MESSAGE}]"), "")
        + &frame("[DONE]");
    stream
        .push(wire.as_bytes(), |event| {
            if let Progress::Reasoning(text) = event {
                reasoning.push_str(text);
            }
            ControlFlow::Continue(())
        })
        .unwrap();
    assert_eq!(reasoning, "Checking");
    assert!(stream.is_finished());
    assert_eq!(
        stream.push(b"", |_| ControlFlow::Continue(())),
        Err(Error::Closed)
    );
    assert_eq!(stream.finish().unwrap().text, "Hi 🚀");
    let unfinished = MESSAGE.replace("completed", "in_progress");
    assert_eq!(
        consume(&terminal("completed", &format!("[{unfinished}]"), "")),
        Err(Error::ConflictingOutput)
    );
    let missing_output = terminal("completed", "[]", "").replace(",\"output\":[]", "");
    assert!(!missing_output.contains("output"));
    assert_eq!(
        consume(&(done(0, MESSAGE) + &missing_output)).unwrap().text,
        "Hi 🚀"
    );
}

#[test]
fn indexed_deltas_preserve_message_and_content_part_boundaries() {
    let first = message(
        "msg_first",
        r#"[{"type":"output_text","text":"Hello world"},{"type":"output_text","text":"Next part"}]"#,
    );
    let second = message(
        "msg_second",
        r#"[{"type":"output_text","text":"Hello world"}]"#,
    );
    let wire = delta(0, 0, "msg_first", "Hello ", 1)
        + &delta(0, 0, "msg_first", "world", 2)
        + &delta(0, 1, "msg_first", "Next part", 3)
        + &delta(1, 0, "msg_second", "Hello world", 4)
        + &terminal("completed", &format!("[{first},{second}]"), "");
    let mut stream = ResponseStream::default();
    let mut shown = String::new();
    stream
        .push(wire.as_bytes(), |progress| {
            if let Progress::Text(text) = progress {
                shown.push_str(text);
            }
            ControlFlow::Continue(())
        })
        .unwrap();
    let response = stream.finish().unwrap();
    assert_eq!(shown, "Hello world\nNext part\n\nHello world");
    assert_eq!(response.text, shown);
    assert_eq!(response.output.len(), 2);
    assert_eq!(
        response.output[0].get("id").unwrap().text(),
        Some("msg_first")
    );
    assert_eq!(
        response.output[1].get("id").unwrap().text(),
        Some("msg_second")
    );
}

#[test]
fn unindexed_progress_reconciles_against_distinct_terminal_messages() {
    let first = message("msg_first", r#"[{"type":"output_text","text":"Same"}]"#);
    let second = message("msg_second", r#"[{"type":"output_text","text":"Same"}]"#);
    let wire = frame(r#"{"type":"response.output_text.delta","delta":"SameSame"}"#)
        + &terminal("completed", &format!("[{first},{second}]"), "");
    let response = consume(&wire).unwrap();
    assert_eq!(response.text, "Same\n\nSame");
    assert_eq!(response.output.len(), 2);
}

#[test]
fn indexed_terminal_suffix_and_conflicting_replays_are_distinguished() {
    let item = message(
        "msg_one",
        r#"[{"type":"output_text","text":"prefix suffix"}]"#,
    );
    let prefix = delta(0, 0, "msg_one", "prefix", 1);
    let result =
        consume(&(prefix.clone() + &terminal("completed", &format!("[{item}]"), ""))).unwrap();
    assert_eq!(result.text, "prefix suffix");
    assert_eq!(
        consume(&(prefix.clone() + &delta(0, 0, "msg_one", "prefix", 1))),
        Err(Error::ConflictingOutput)
    );
    assert_eq!(
        consume(&(prefix.clone() + &delta(0, 0, "other", " suffix", 2))),
        Err(Error::ConflictingOutput)
    );
    let done_text = frame(
        r#"{"type":"response.output_text.done","output_index":0,"content_index":0,"item_id":"msg_one","text":"prefix suffix"}"#,
    );
    let done_part = frame(
        r#"{"type":"response.content_part.done","output_index":0,"content_index":0,"item_id":"msg_one","part":{"type":"output_text","text":"prefix suffix"}}"#,
    );
    let completed = terminal("completed", &format!("[{item}]"), "");
    assert_eq!(
        consume(&(prefix.clone() + &done_text + &done_part + &completed))
            .unwrap()
            .text,
        "prefix suffix"
    );
    assert_eq!(
        consume(&(prefix.clone() + &done_text + &done_text + &completed)),
        Err(Error::ConflictingOutput)
    );
    assert_eq!(
        consume(&(prefix + &done_text.replace("prefix suffix", "conflict") + &completed)),
        Err(Error::ConflictingOutput)
    );
}

#[test]
fn terminal_can_supply_a_suffix_inside_an_earlier_content_part() {
    let item = message(
        "msg_one",
        r#"[{"type":"output_text","text":"first suffix"},{"type":"output_text","text":"second"}]"#,
    );
    let wire = delta(0, 0, "msg_one", "first", 1)
        + &delta(0, 1, "msg_one", "second", 2)
        + &terminal("completed", &format!("[{item}]"), "");
    let mut stream = ResponseStream::default();
    let mut shown = String::new();
    stream
        .push(wire.as_bytes(), |progress| {
            if let Progress::Text(text) = progress {
                shown.push_str(text);
            }
            ControlFlow::Continue(())
        })
        .unwrap();
    assert_eq!(shown, "first\nsecond");
    assert_eq!(stream.finish().unwrap().text, "first suffix\nsecond");
}
