use super::completion::parse_completion;
use super::*;
use crate::json;
use crate::test_support::HttpFixture;

#[test]
fn context_capacity_comes_from_the_exact_model_and_is_cached() {
    let fixture = HttpFixture::new(vec![(
        200,
        Value::object([(
            "data",
            Value::Array(vec![
                Value::object([
                    ("id", Value::string("fixture/other")),
                    ("context_length", Value::number(999999)),
                ]),
                Value::object([
                    ("id", Value::string("fixture/model")),
                    ("context_length", Value::number(32768)),
                    (
                        "top_provider",
                        Value::object([("max_completion_tokens", Value::number(4096))]),
                    ),
                ]),
            ]),
        )]),
    )]);
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    client.limits = None;
    let limits = client.limits().unwrap().unwrap();
    assert_eq!(limits.context, 32768);
    assert_eq!(limits.output, Some(4096));
    assert_eq!(client.limits().unwrap().unwrap().context, 32768);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].headers[0].starts_with("GET /models?"));
}

#[test]
fn native_http_sends_json_and_bearer_auth() {
    let fixture = HttpFixture::new(vec![(200, json::parse(r#"{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"Hello"}}]}"#).unwrap())]);
    let client = OpenRouter::fixture(fixture.endpoint.clone());
    let messages = vec![Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Quote: \"; newline:\n; café 🦀")),
    ])];
    assert_eq!(client.complete(&messages).unwrap().text, "Hello");
    let requests = fixture.finish();
    assert_eq!(
        requests[0].body.get("messages"),
        Some(&Value::Array(messages))
    );
    assert_eq!(
        requests[0]
            .body
            .get("tools")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        crate::tools::definitions().as_array().unwrap().len()
    );
    assert_eq!(
        requests[0]
            .body
            .get("tools")
            .and_then(Value::as_array)
            .unwrap()[0]
            .get("function")
            .and_then(|function| function.get("parameters"))
            .and_then(|parameters| parameters.get("properties"))
            .and_then(|properties| properties.get("work"))
            .and_then(|work| work.get("type"))
            .and_then(Value::as_str),
        Some("string")
    );
    assert!(
        requests[0]
            .headers
            .iter()
            .any(|header| header == "Authorization: Bearer isolated-fixture-key")
    );
    assert!(
        requests[0]
            .headers
            .iter()
            .any(|header| header == "X-OpenRouter-Metadata: enabled")
    );
}

#[test]
fn automatic_cache_control_is_scoped_to_claude_agent_requests() {
    let response = crate::test_support::completion("ok", vec![]);
    let fixture = HttpFixture::new(vec![
        (200, response.clone()),
        (200, response.clone()),
        (200, response),
    ]);
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    client
        .set_model("anthropic/claude-sonnet-4.5".into())
        .unwrap();
    client.complete(&[]).unwrap();
    client.sample(&[], false, None, &mut |_| Ok(())).unwrap();
    client.set_model("openai/gpt-5".into()).unwrap();
    client.complete(&[]).unwrap();
    let requests = fixture.finish();
    assert_eq!(
        requests[0].body.get("cache_control"),
        Some(&Value::object([("type", Value::string("ephemeral"))]))
    );
    assert!(requests[0].body.get("tools").is_some());
    assert!(requests[1].body.get("cache_control").is_none());
    assert!(requests[1].body.get("tools").is_none());
    assert!(requests[2].body.get("cache_control").is_none());
}

#[test]
fn automatic_cache_control_accepts_claude_alias_only_for_agent_requests() {
    let response = crate::test_support::completion("ok", vec![]);
    let fixture = HttpFixture::new(vec![
        (200, response.clone()),
        (200, response.clone()),
        (200, response),
    ]);
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    let alias = "~anthropic/claude-sonnet-latest";
    client.set_model(alias.into()).unwrap();
    client.complete(&[]).unwrap();
    client.sample(&[], false, None, &mut |_| Ok(())).unwrap();
    client.set_model("~openai/gpt-latest".into()).unwrap();
    client.complete(&[]).unwrap();

    let requests = fixture.finish();
    assert_eq!(requests[0].body.get("model"), Some(&Value::string(alias)));
    assert_eq!(
        requests[0].body.get("cache_control"),
        Some(&Value::object([("type", Value::string("ephemeral"))]))
    );
    assert!(requests[0].body.get("tools").is_some());
    assert!(requests[1].body.get("cache_control").is_none());
    assert!(requests[1].body.get("tools").is_none());
    assert!(requests[2].body.get("cache_control").is_none());
    assert!(requests[2].body.get("tools").is_some());
}

#[test]
fn request_effort_is_transient_and_internal_state_marker_is_not_sent() {
    let fixture = HttpFixture::new(vec![(200, crate::test_support::completion("ok", vec![]))]);
    let client = OpenRouter::fixture(fixture.endpoint.clone());
    let message = Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Current request state")),
        ("jecode_request_state", Value::Bool(true)),
    ]);
    let completion = crate::backend::Backend::sample_with_effort(
        &client,
        Effort::Medium,
        &[message],
        false,
        None,
        None,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(completion.text, "ok");
    assert_eq!(client.effort(), Effort::Default);
    let requests = fixture.finish();
    assert_eq!(
        requests[0]
            .body
            .get("reasoning")
            .and_then(|value| value.get("effort"))
            .and_then(Value::as_str),
        Some("medium")
    );
    assert!(!requests[0].body.encode().contains("jecode_request_state"));
}

#[test]
fn claude_cache_stops_before_request_only_state() {
    let fixture = HttpFixture::new(vec![(200, crate::test_support::completion("ok", vec![]))]);
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    client
        .set_model("anthropic/claude-sonnet-4.5".into())
        .unwrap();
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
    crate::backend::Backend::sample(&client, &messages, true, None, None, &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    assert!(requests[0].body.get("cache_control").is_none());
    let history = requests[0]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap();
    assert!(
        history[0].get("content").and_then(Value::as_array).unwrap()[0]
            .get("cache_control")
            .is_some()
    );
    assert_eq!(
        history[1].get("content"),
        Some(&Value::string("Changing state"))
    );
}

#[test]
fn growing_tool_batches_keep_the_prior_explicit_boundary() {
    fn batch(messages: &mut Vec<Value>, number: usize) {
        messages.push(Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("Tool calls")),
        ]));
        for index in 0..24 {
            messages.push(Value::object([
                ("role", Value::string("tool")),
                (
                    "tool_call_id",
                    Value::string(format!("call_{number}_{index}")),
                ),
                ("content", Value::string(format!("Result {number}:{index}"))),
            ]));
        }
    }
    for model in ["anthropic/claude-sonnet-4.5", "openai/gpt-6.1-sol"] {
        let completion = crate::test_support::completion("ok", vec![]);
        let fixture = HttpFixture::new(vec![(200, completion.clone()), (200, completion)]);
        let mut client = OpenRouter::fixture(fixture.endpoint.clone());
        client.set_model(model.into()).unwrap();
        let mut messages = vec![Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Stable task")),
        ])];
        batch(&mut messages, 0);
        let state = Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Changing request state")),
        ]);
        let mut first = messages.clone();
        first.push(state.clone());
        client
            .sample_owned(first, Effort::Default, true, true, None, &mut |_| Ok(()))
            .unwrap();
        batch(&mut messages, 1);
        messages.push(state);
        client
            .sample_owned(messages, Effort::Default, true, true, None, &mut |_| Ok(()))
            .unwrap();
        let requests = fixture.finish();
        let first = requests[0]
            .body
            .get("messages")
            .and_then(Value::as_array)
            .unwrap();
        let second = requests[1]
            .body
            .get("messages")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(
            &first[..26],
            &second[..26],
            "{model} changed the cached prefix"
        );
        for index in [25, 50] {
            assert!(
                second[index]
                    .get("content")
                    .and_then(Value::as_array)
                    .unwrap()[0]
                    .get("cache_control")
                    .is_some()
            );
        }
        assert!(
            requests
                .iter()
                .all(|request| request.body.get("cache_control").is_none())
        );
    }
}

#[test]
fn provider_errors_do_not_expose_credentials() {
    let fixture = HttpFixture::new(vec![(
        401,
        Value::object([(
            "error",
            Value::object([("message", Value::string("Rejected isolated-fixture-key"))]),
        )]),
    )]);
    let error = OpenRouter::fixture(fixture.endpoint.clone())
        .complete(&[])
        .err()
        .unwrap();
    assert!(error.contains("HTTP 401"));
    assert!(error.contains("[redacted]"));
    assert!(!error.contains("isolated-fixture-key"));
    fixture.finish();
}

#[test]
fn transport_has_no_fixed_four_megabyte_conversation_limit() {
    let messages = [Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("x".repeat(4 * 1024 * 1024))),
    ])];
    // Encoding and escaping this request can exceed the fixture's usual wait on busy runners.
    let fixture = HttpFixture::with_wait(
        vec![(200, crate::test_support::completion("Accepted.", vec![]))],
        std::time::Duration::from_secs(30),
    );
    let client = OpenRouter::fixture(fixture.endpoint.clone());
    assert_eq!(client.complete(&messages).unwrap().text, "Accepted.");
    assert_eq!(
        fixture.finish()[0].body.get("messages"),
        Some(&Value::Array(messages.to_vec()))
    );
}

#[test]
fn incomplete_and_malformed_tool_calls_are_rejected_before_execution() {
    for response in [
        r#"{"choices":[{"finish_reason":"length","message":{"role":"assistant","content":"partial"}}]}"#,
        r#"{"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"tool_calls":[{"id":"x","type":"function","function":{"name":"read","arguments":"{}"}},{"id":"x","type":"function","function":{"name":"write","arguments":"{}"}}]}}]}"#,
        r#"{"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","tool_calls":[]}}]}"#,
    ] {
        assert!(parse_completion(json::parse(response).unwrap()).is_err());
    }
    assert!(OpenRouter::new("bad\nheader".into(), "fixture/model".into()).is_err());
}
