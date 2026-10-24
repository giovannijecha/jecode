use super::*;
use crate::test_support::{Directory, HttpFixture, Response};
use std::time::Duration;

#[test]
fn request_timing_records_dispatch_and_first_model_update_without_payload() {
    let fixture = HttpFixture::streaming(vec![Response::Stream(vec![(
        Duration::from_millis(5),
        "data: {\"choices\":[{\"delta\":{\"content\":\"Reply marker\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into(),
    )])]);
    let directory = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let mut phases = Vec::new();
    agent
        .run_turn("Prompt marker", &mut |event| {
            match event {
                Event::Preparing => phases.push("preparing"),
                Event::Waiting { .. } => phases.push("waiting"),
                Event::Streaming { .. } => phases.push("streaming"),
                _ => {}
            }
            Ok(())
        })
        .unwrap();
    assert!(
        phases[..phases.len() - 2]
            .iter()
            .all(|phase| *phase == "preparing")
    );
    assert_eq!(&phases[phases.len() - 2..], ["waiting", "streaming"]);
    let request = fixture.finish().remove(0);
    let document = agent.archive().document();
    let timing = document
        .get("events")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|event| event.get("type").and_then(Value::as_str) == Some("request_timing"))
        .unwrap();
    assert_eq!(
        timing.get("operation").and_then(Value::as_str),
        Some("completion")
    );
    assert_eq!(
        timing.get("status").and_then(Value::as_str),
        Some("complete")
    );
    assert_eq!(
        timing.get("backend").and_then(Value::as_str),
        Some("openrouter")
    );
    assert_eq!(
        timing.get("request_bytes").and_then(Value::as_usize),
        Some(request.body.encode().len())
    );
    assert!(timing.get("started_at_unix_ms").is_some());
    assert!(
        timing
            .get("preparation_ms")
            .and_then(Value::as_usize)
            .is_some()
    );
    let first = timing
        .get("first_update_ms")
        .and_then(Value::as_usize)
        .unwrap();
    let response = timing.get("response_ms").and_then(Value::as_usize).unwrap();
    assert!(first <= response);
    assert!(!timing.encode().contains("Prompt marker"));
    assert!(!timing.encode().contains("Reply marker"));
}
