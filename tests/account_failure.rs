use jecode::providers::openai_account::{
    Error, FailureCode, FailureEvent, Progress, ProviderFailure, Response, ResponseStream,
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
fn terminal(status: &str, output: &str, extra: &str) -> String {
    frame(&format!(
        r#"{{"type":"response.{status}","response":{{"id":"resp_test","status":"{status}","output":{output}{extra}}}}}"#
    ))
}
fn consume(text: &str) -> Result<Response, Error> {
    let mut stream = ResponseStream::default();
    stream.push(text.as_bytes(), |_: Progress<'_>| ControlFlow::Continue(()))?;
    stream.finish()
}
const TOOL: &str = r#"{"type":"function_call","id":"fc_1","status":"completed","call_id":"call_1","name":"read_file","arguments":"{\"path\":\"README.md\"}"}"#;

#[test]
fn provider_failure_events_keep_their_distinct_classification() {
    let failed = consume(&frame(
        r#"{"type":"response.failed","response":{"id":"resp_fixture","status":"failed","error":{"code":"server_error","message":"secret-like fixture"}}}"#,
    ))
    .unwrap_err();
    let error = consume(&frame(
        r#"{"type":"error","code":"rate_limit_exceeded","message":"secret-like fixture"}"#,
    ))
    .unwrap_err();
    assert_ne!(failed, error);
    assert_eq!(
        failed,
        Error::RemoteFailure(ProviderFailure {
            event: FailureEvent::ResponseFailed,
            code: FailureCode::ServerError,
        })
    );
    assert_eq!(
        error,
        Error::RemoteFailure(ProviderFailure {
            event: FailureEvent::Error,
            code: FailureCode::RateLimitExceeded,
        })
    );
    assert!(!failed.to_string().contains("secret-like fixture"));
    assert!(!error.to_string().contains("secret-like fixture"));
}

#[test]
fn provider_failure_classification_is_bounded_and_never_uses_messages() {
    let cases = [
        (
            r#"{"type":"response.failed"}"#.to_owned(),
            FailureEvent::ResponseFailed,
            FailureCode::Unknown,
        ),
        (
            r#"{"type":"response.failed","response":{"error":null}}"#.to_owned(),
            FailureEvent::ResponseFailed,
            FailureCode::Unknown,
        ),
        (
            r#"{"type":"response.failed","response":{"error":[]}}"#.to_owned(),
            FailureEvent::ResponseFailed,
            FailureCode::Malformed,
        ),
        (
            r#"{"type":"error"}"#.to_owned(),
            FailureEvent::Error,
            FailureCode::Unknown,
        ),
        (
            r#"{"type":"error","error":{"code":"server_error"}}"#.to_owned(),
            FailureEvent::Error,
            FailureCode::Unknown,
        ),
        (
            r#"{"type":"response.failed","code":"server_error","response":{"error":null}}"#
                .to_owned(),
            FailureEvent::ResponseFailed,
            FailureCode::Unknown,
        ),
        (
            r#"{"type":"error","code":"private-secret-code"}"#.to_owned(),
            FailureEvent::Error,
            FailureCode::Unknown,
        ),
        (
            r#"{"type":"error","code":15}"#.to_owned(),
            FailureEvent::Error,
            FailureCode::Malformed,
        ),
        (
            format!(r#"{{"type":"error","code":"{}"}}"#, "x".repeat(65)),
            FailureEvent::Error,
            FailureCode::Malformed,
        ),
        (
            r#"{"type":"error","code":"server_is_overloaded"}"#.to_owned(),
            FailureEvent::Error,
            FailureCode::ServerIsOverloaded,
        ),
        (
            r#"{"type":"response.failed","response":{"error":{"code":"context_length_exceeded"}}}"#
                .to_owned(),
            FailureEvent::ResponseFailed,
            FailureCode::ContextLengthExceeded,
        ),
    ];
    for (payload, event, code) in cases {
        let wire = frame(&payload);
        assert_eq!(
            consume(&wire),
            Err(Error::RemoteFailure(ProviderFailure { event, code }))
        );
    }
    let sentinel = "synthetic-secret-token";
    let wire = frame(&format!(
        r#"{{"type":"error","code":"rate_limit_exceeded","message":"{}"}}"#,
        sentinel.repeat(1000)
    ));
    let error = consume(&wire).unwrap_err();
    assert!(!format!("{error:?} {error}").contains(sentinel));
    assert_eq!(
        error,
        Error::RemoteFailure(ProviderFailure {
            event: FailureEvent::Error,
            code: FailureCode::RateLimitExceeded,
        })
    );
}

#[test]
fn failed_stream_cannot_complete_a_partial_tool_but_validated_completion_survives() {
    let failure = frame(r#"{"type":"error","code":"rate_limit_exceeded"}"#);
    assert_eq!(
        consume(&(done(0, TOOL) + &failure)),
        Err(Error::RemoteFailure(ProviderFailure {
            event: FailureEvent::Error,
            code: FailureCode::RateLimitExceeded,
        }))
    );
    let completed = consume(&(terminal("completed", &format!("[{TOOL}]"), "") + &failure)).unwrap();
    assert_eq!(completed.tool_calls.len(), 1);
}
