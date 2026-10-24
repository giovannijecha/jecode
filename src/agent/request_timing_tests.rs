use super::*;
use crate::test_support::{Directory, HttpFixture, Response};
use std::time::Duration;

#[test]
fn dispatch_notification_wait_is_separate_from_local_and_remote_work() {
    let fixture = HttpFixture::new(vec![(
        200,
        crate::test_support::completion("Done.", vec![]),
    )]);
    let project = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent
        .run_turn("Measure a slow event consumer", &mut |event| {
            if matches!(event, Event::Waiting { .. }) {
                std::thread::sleep(Duration::from_millis(40));
            }
            Ok(())
        })
        .unwrap();
    fixture.finish();
    let document = agent.archive().document();
    let event = document
        .get("events")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|event| event.get("type").and_then(Value::as_str) == Some("request_timing"))
        .unwrap();
    let ms = |name| event.get(name).and_then(Value::as_usize).unwrap();
    assert!(ms("notification_ms") >= 40);
    let parts = ms("preparation_ms") + ms("notification_ms") + ms("response_ms");
    assert!(parts <= ms("total_ms"));
    assert!(
        ms("total_ms") - parts <= 2,
        "only millisecond rounding may separate the measured phases"
    );
}

#[test]
fn cancelled_attempt_has_its_own_export_status() {
    // Cancellation in the dispatch notification must prevent any HTTP request.
    let fixture = HttpFixture::new(vec![]);
    let project = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    let cancellation = agent.cancellation();
    let error = agent
        .run_turn("Stop before sending", &mut |event| {
            if matches!(event, Event::Waiting { .. }) {
                cancellation.cancel();
            }
            Ok(())
        })
        .unwrap_err();
    assert!(error.contains("cancelled"));
    assert!(fixture.finish().is_empty());
    let document = agent.archive().document();
    let event = document
        .get("events")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|event| event.get("type").and_then(Value::as_str) == Some("request_timing"))
        .unwrap();
    assert_eq!(
        event.get("status").and_then(Value::as_str),
        Some("cancelled")
    );
    for field in ["preparation_ms", "first_update_ms", "response_ms"] {
        assert_eq!(
            event.get(field),
            Some(&Value::Null),
            "{field}: no HTTP attempt was dispatched"
        );
    }
}

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
    assert_eq!(phases, ["preparing", "waiting", "streaming"]);
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
