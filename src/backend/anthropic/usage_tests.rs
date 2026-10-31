use super::*;
use crate::backend::Update;

#[test]
fn thinking_usage_is_emitted_before_a_failed_stream_ends() {
    let mut stream = stream::Stream::default();
    let event = crate::json::parse(r#"{"type":"message_start","message":{"usage":{"input_tokens":40,"output_tokens":8,"output_tokens_details":{"thinking_tokens":6}}}}"#).unwrap();
    let data = format!("data: {}\n\n", event.encode());
    let mut reported = Vec::new();
    stream
        .feed(data.as_bytes(), &mut |update| {
            if let Update::Usage(usage) = update {
                reported.push(usage);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(reported[0].reasoning_tokens, Some(6));
    assert!(stream.finish().is_err());
}
