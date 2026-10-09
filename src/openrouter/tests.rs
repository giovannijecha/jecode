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
fn native_curl_sends_json_and_bearer_auth() {
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
        4
    );
    assert!(
        requests[0]
            .headers
            .iter()
            .any(|header| header == "Authorization: Bearer isolated-fixture-key")
    );
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
