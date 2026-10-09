use super::*;
use crate::test_support::HttpFixture;

fn provider_error(message: &str, code: &str, param: &str) -> Value {
    Value::object([(
        "error",
        Value::object([
            ("message", Value::string("Provider returned error")),
            ("code", Value::number(400)),
            (
                "metadata",
                Value::object([
                    ("provider_name", Value::string("OpenAI")),
                    ("provider_error_code", Value::string(code)),
                    (
                        "raw",
                        Value::string(
                            Value::object([(
                                "error",
                                Value::object([
                                    ("message", Value::string(message)),
                                    ("code", Value::string(code)),
                                    ("param", Value::string(param)),
                                ]),
                            )])
                            .encode(),
                        ),
                    ),
                ]),
            ),
        ]),
    )])
}

#[test]
fn provider_regression_error_identifies_invalid_tool_parameters() {
    let failure = Failure::api(
        &provider_error(
            "A numeric value in the function parameters is too large.",
            "invalid_function_parameters",
            "tools[0].parameters",
        ),
        400,
    );
    assert_eq!(failure.kind, Kind::Terminal);
    for expected in [
        "OpenAI",
        "numeric value",
        "invalid_function_parameters",
        "tools[0].parameters",
    ] {
        assert!(
            failure.message.contains(expected),
            "missing {expected}: {}",
            failure.message
        );
    }
}

#[test]
fn provider_regression_nested_context_error_uses_context_recovery() {
    let failure = Failure::api(
        &provider_error(
            "The maximum context length was exceeded.",
            "context_length_exceeded",
            "messages",
        ),
        400,
    );
    assert_eq!(failure.kind, Kind::Context);
}

#[test]
fn provider_regression_nested_error_keeps_diagnostics_and_redacts_credentials() {
    let fixture = HttpFixture::new(vec![(
        400,
        provider_error(
            "Rejected isolated-fixture-key in the supplied parameters.",
            "invalid_function_parameters",
            "tools[0].parameters",
        ),
    )]);
    let error = crate::openrouter::OpenRouter::fixture(fixture.endpoint.clone())
        .complete(&[])
        .err()
        .unwrap();
    assert!(error.contains("supplied parameters"));
    assert!(error.contains("[redacted]"));
    assert!(!error.contains("isolated-fixture-key"));
    fixture.finish();
}

#[test]
fn provider_regression_long_diagnostics_are_redacted_before_utf8_clipping() {
    let api = crate::openrouter::Api::fixture("http://127.0.0.1:1");
    let diagnostic = format!(
        "{}isolated-fixture-key{}",
        "x".repeat(8000),
        "λ".repeat(2000)
    );
    let failure = api.failure(
        &provider_error(&diagnostic, "invalid_request", "tools"),
        400,
    );
    assert!(failure.message.ends_with("[provider diagnostic truncated]"));
    assert!(failure.message.len() < 8300);
    assert!(!failure.message.contains("isolated-fixture"));
    assert_eq!(failure.kind, Kind::Terminal);
}
