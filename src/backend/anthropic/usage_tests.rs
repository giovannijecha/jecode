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

#[test]
fn cache_creation_breakdown_survives_a_later_usage_delta() {
    let mut stream = stream::Stream::default();
    let events = concat!(
        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":7,\"cache_read_input_tokens\":3,\"cache_creation_input_tokens\":30,\"cache_creation\":{\"ephemeral_5m_input_tokens\":10,\"ephemeral_1h_input_tokens\":20},\"output_tokens\":1}}}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{},\"usage\":{\"output_tokens\":9}}\n\n",
    );
    let mut reported = Vec::new();
    stream
        .feed(events.as_bytes(), &mut |update| {
            if let Update::Usage(usage) = update {
                reported.push(usage);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(reported.len(), 2);
    let final_usage = reported[1];
    assert_eq!(final_usage.input_tokens, Some(40));
    assert_eq!(final_usage.output_tokens, Some(9));
    assert_eq!(final_usage.cache_write_tokens, Some(30));
    assert_eq!(final_usage.cache_write_5m_tokens, Some(10));
    assert_eq!(final_usage.cache_write_1h_tokens, Some(20));
}
