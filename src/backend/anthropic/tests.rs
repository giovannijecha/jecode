use super::*;
use crate::backend::{FailureKind, FailureReason, Update};
use crate::test_support::{HttpFixture, Response};
use std::time::Duration;

fn event(value: Value) -> String {
    format!("data: {}\n\n", value.encode())
}

fn stream(blocks: Vec<Value>, deltas: Vec<Value>, reason: &str) -> Response {
    let mut data = event(Value::object([
        ("type", Value::string("message_start")),
        ("message", Value::object([("usage", crate::json::parse(r#"{"input_tokens":5,"cache_read_input_tokens":3,"cache_creation_input_tokens":2,"output_tokens":1}"#).unwrap())])),
    ]));
    for (index, block) in blocks.into_iter().enumerate() {
        data.push_str(&event(Value::object([
            ("type", Value::string("content_block_start")),
            ("index", Value::number(index)),
            ("content_block", block),
        ])));
        for delta in deltas
            .iter()
            .filter(|delta| delta.get("index").and_then(Value::as_usize) == Some(index))
        {
            data.push_str(&event(delta.clone()));
        }
        data.push_str(&event(Value::object([
            ("type", Value::string("content_block_stop")),
            ("index", Value::number(index)),
        ])));
    }
    data.push_str(&event(Value::object([
        ("type", Value::string("message_delta")),
        (
            "delta",
            Value::object([("stop_reason", Value::string(reason))]),
        ),
        (
            "usage",
            Value::object([("output_tokens", Value::number(9))]),
        ),
    ])));
    data.push_str(&event(Value::object([(
        "type",
        Value::string("message_stop"),
    )])));
    Response::Stream(vec![(Duration::ZERO, data)])
}

fn delta(index: usize, kind: &str, key: &str, value: &str) -> Value {
    Value::object([
        ("type", Value::string("content_block_delta")),
        ("index", Value::number(index)),
        (
            "delta",
            Value::object([("type", Value::string(kind)), (key, Value::string(value))]),
        ),
    ])
}

fn client(fixture: &HttpFixture) -> Box<dyn Backend> {
    let endpoint = fixture.endpoint.trim_end_matches("/chat/completions");
    Api::fixture(Session::fixture("fixture-anthropic-token"), endpoint)
        .create("claude-opus-5-5".into())
        .unwrap()
}

#[test]
fn catalog_reads_account_limits_and_effort_capabilities() {
    let catalog = crate::json::parse(r#"{"data":[{"id":"claude-opus-5-5","display_name":"Claude Opus 5.5","max_input_tokens":1000000,"max_tokens":128000,"capabilities":{"effort":{"low":{"supported":true},"medium":{"supported":true},"high":{"supported":true},"xhigh":{"supported":true},"max":{"supported":false}},"thinking":{"types":{"between_tools":{"supported":true},"disabled":{"supported":false}}}}}],"has_more":false}"#).unwrap();
    let fixture = HttpFixture::new(vec![(200, catalog)]);
    let api = Api::fixture(
        Session::fixture("fixture-anthropic-token"),
        fixture.endpoint.trim_end_matches("/chat/completions"),
    );
    let models = api.catalog().unwrap();
    assert_eq!(
        models[0].efforts,
        [
            Effort::Default,
            Effort::Low,
            Effort::Medium,
            Effort::High,
            Effort::Xhigh,
            Effort::None
        ]
    );
    assert_eq!(
        api.known_limits("claude-opus-5-5").unwrap().context,
        1_000_000
    );
    assert_eq!(
        api.known_limits("claude-opus-5-5").unwrap().output,
        Some(128_000)
    );
    assert_eq!(api.thinking_off("claude-opus-5-5"), Some("between_tools"));
    let requests = fixture.finish();
    assert!(requests[0].headers[0].starts_with("GET /v1/models?limit=100 "));
    assert!(
        requests[0]
            .headers
            .iter()
            .any(|line| line.eq_ignore_ascii_case("authorization: Bearer fixture-anthropic-token"))
    );
    assert!(
        requests[0]
            .headers
            .iter()
            .any(|line| line.eq_ignore_ascii_case("user-agent: jecode/0.1"))
    );
}

#[test]
fn native_tool_stream_replays_signed_thinking_for_same_account_and_model() {
    let first = stream(
        vec![
            Value::object([
                ("type", Value::string("thinking")),
                ("thinking", Value::string("")),
            ]),
            Value::object([
                ("type", Value::string("tool_use")),
                ("id", Value::string("toolu_1")),
                ("name", Value::string("read")),
                ("input", Value::object([])),
            ]),
        ],
        vec![
            delta(0, "thinking_delta", "thinking", "Consider file"),
            delta(0, "signature_delta", "signature", "signed-block"),
            delta(1, "input_json_delta", "partial_json", "{\"path\":"),
            delta(1, "input_json_delta", "partial_json", "\"README.md\"}"),
        ],
        "tool_use",
    );
    let second = stream(
        vec![Value::object([
            ("type", Value::string("text")),
            ("text", Value::string("")),
        ])],
        vec![delta(0, "text_delta", "text", "File reviewed")],
        "end_turn",
    );
    let fixture = HttpFixture::streaming(vec![first, second]);
    let backend = client(&fixture);
    let user = Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Read README.md")),
    ]);
    let mut updates = Vec::new();
    let first = backend
        .sample(
            std::slice::from_ref(&user),
            true,
            Some(100),
            None,
            &mut |update| {
                if let Update::Usage(usage) = update {
                    updates.push(usage);
                }
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(first.calls[0].id, "toolu_1");
    assert_eq!(first.calls[0].arguments, "{\"path\":\"README.md\"}");
    assert_eq!(first.usage.unwrap().input_tokens, Some(10));
    assert_eq!(first.usage.unwrap().output_tokens, Some(9));
    assert_eq!(updates[0].input_tokens, Some(10));
    assert_eq!(updates[0].output_tokens, Some(1));
    assert_eq!(updates.last().unwrap().output_tokens, Some(9));
    let result = Value::object([
        ("role", Value::string("tool")),
        ("tool_call_id", Value::string("toolu_1")),
        ("content", Value::string("# README")),
    ]);
    let final_answer = backend
        .sample(
            &[user, first.message.clone(), result],
            true,
            Some(100),
            None,
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(final_answer.text, "File reviewed");
    let requests = fixture.finish();
    let first_request = &requests[0].body;
    assert!(requests[0].headers[0].starts_with("POST /v1/messages?beta=true "));
    assert_eq!(
        first_request
            .get("system")
            .and_then(Value::as_array)
            .unwrap()[0]
            .get("text")
            .and_then(Value::as_str)
            .unwrap(),
        "You are Claude Code, Anthropic's official CLI for Claude."
    );
    assert!(requests[0].headers.iter().any(|line| {
        line.eq_ignore_ascii_case("anthropic-beta: claude-code-20250219,oauth-2025-04-20")
    }));
    assert!(
        requests[0]
            .headers
            .iter()
            .any(|line| line.eq_ignore_ascii_case("user-agent: claude-cli/2.1.314"))
    );
    let second_messages = requests[1]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    let blocks = second_messages[1]
        .get("content")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        blocks[0].get("signature").and_then(Value::as_str),
        Some("signed-block")
    );
    assert_eq!(blocks[1].get("id").and_then(Value::as_str), Some("toolu_1"));
    let tool_result = second_messages[2]
        .get("content")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        tool_result[0].get("tool_use_id").and_then(Value::as_str),
        Some("toolu_1")
    );
    assert_eq!(
        tool_result[0].get("content").and_then(Value::as_str),
        Some("# README")
    );
    assert!(!requests[1].body.encode().contains("jecode_origin"));
    let other = Origin::with_account(
        "anthropic",
        "claude-opus-5-5",
        Some("different-account".into()),
    );
    let projected = conversation::project(&first.message, &other);
    assert!(projected.get("anthropic_blocks").is_none());
    let portable = wire::request(
        "claude-opus-5-5",
        Effort::Default,
        &[projected],
        None,
        None,
        None,
        None,
    )
    .unwrap();
    assert!(!portable.encode().contains("signed-block"));
}

#[test]
fn rate_limit_and_retry_header_are_preserved() {
    let fixture = HttpFixture::streaming(vec![Response::Http(
        429,
        vec![
            ("Retry-After".into(), "90".into()),
            ("Request-Id".into(), "req_fixture".into()),
        ],
        crate::json::parse(
            r#"{"type":"error","error":{"type":"rate_limit_error","message":"Wait"}}"#,
        )
        .unwrap(),
    )]);
    let backend = client(&fixture);
    let failure = backend
        .sample(&[], false, None, None, &mut |_| Ok(()))
        .err()
        .unwrap();
    assert_eq!(failure.kind, FailureKind::Temporary);
    assert_eq!(failure.reason, Some(FailureReason::RateLimited));
    assert_eq!(failure.retry_after, Some(Duration::from_secs(90)));
    assert!(failure.message.contains("req_fixture"));
    fixture.finish();
}

#[test]
fn wire_preserves_substantive_instructions_and_neutral_tool_schema() {
    let messages = vec![
        Value::object([
            ("role", Value::string("system")),
            ("content", Value::string("Keep all project instructions.")),
        ]),
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Proceed")),
        ]),
    ];
    let request = wire::request(
        "claude-opus-5-5",
        Effort::High,
        &messages,
        Some(crate::tools::definitions_for(false)),
        Some(777),
        None,
        None,
    )
    .unwrap();
    let system = request.get("system").and_then(Value::as_array).unwrap();
    assert_eq!(
        system[0].get("text").and_then(Value::as_str),
        Some("You are Claude Code, Anthropic's official CLI for Claude.")
    );
    assert_eq!(
        system[1].get("text").and_then(Value::as_str),
        Some("Keep all project instructions.")
    );
    assert_eq!(
        request.get("max_tokens").and_then(Value::as_usize),
        Some(777)
    );
    assert_eq!(
        request
            .get("output_config")
            .unwrap()
            .get("effort")
            .and_then(Value::as_str),
        Some("high")
    );
    let tool = &request.get("tools").unwrap().as_array().unwrap()[0];
    assert!(tool.get("input_schema").is_some());
    assert!(tool.get("parameters").is_none());
}

#[test]
fn thinking_off_uses_catalog_mode_and_xhigh_is_sent_exactly() {
    let user = Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Hello")),
    ]);
    let off = wire::request(
        "model-a",
        Effort::None,
        std::slice::from_ref(&user),
        None,
        None,
        Some("between_tools"),
        None,
    )
    .unwrap();
    assert_eq!(
        off.get("thinking")
            .unwrap()
            .get("type")
            .and_then(Value::as_str),
        Some("between_tools")
    );
    assert!(
        wire::request(
            "model-a",
            Effort::None,
            std::slice::from_ref(&user),
            None,
            None,
            None,
            None
        )
        .is_err()
    );
    let high = wire::request("model-a", Effort::Xhigh, &[user], None, None, None, None).unwrap();
    assert_eq!(
        high.get("output_config")
            .unwrap()
            .get("effort")
            .and_then(Value::as_str),
        Some("xhigh")
    );
}

#[test]
fn request_effort_overrides_selection_without_changing_it() {
    let fixture = HttpFixture::streaming(vec![stream(
        vec![Value::object([
            ("type", Value::string("text")),
            ("text", Value::string("")),
        ])],
        vec![delta(0, "text_delta", "text", "Done")],
        "end_turn",
    )]);
    let backend = client(&fixture);
    let result = backend
        .sample_with_effort(Effort::Medium, &[], false, Some(64), None, &mut |_| Ok(()))
        .unwrap();
    assert_eq!(result.text, "Done");
    assert_eq!(backend.effort(), Effort::Default);
    let requests = fixture.finish();
    assert_eq!(
        requests[0]
            .body
            .get("output_config")
            .and_then(|config| config.get("effort"))
            .and_then(Value::as_str),
        Some("medium")
    );
}

#[test]
fn incomplete_tool_block_never_becomes_an_executable_empty_call() {
    let mut stream = stream::Stream::default();
    let input = format!(
        "{}{}{}",
        event(Value::object([
            ("type", Value::string("content_block_start")),
            ("index", Value::number(0)),
            (
                "content_block",
                Value::object([
                    ("type", Value::string("tool_use")),
                    ("id", Value::string("toolu_partial")),
                    ("name", Value::string("read")),
                    ("input", Value::object([])),
                ])
            ),
        ])),
        event(delta(0, "input_json_delta", "partial_json", "{\"path\":")),
        event(Value::object([("type", Value::string("message_stop"))])),
    );
    for chunk in input.as_bytes().chunks(3) {
        stream.feed(chunk, &mut |_| Ok(())).unwrap();
    }
    let failure = stream.finish().err().unwrap();
    assert_eq!(failure.kind, FailureKind::Temporary);
    assert!(failure.message.contains("content block"));
}
