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

#[test]
fn compaction_attempt_uses_medium_without_changing_the_selected_effort() {
    let mut reply = crate::test_support::completion("Summary", vec![]);
    if let Value::Object(fields) = &mut reply {
        fields.insert(
            "usage".into(),
            Value::object([
                ("prompt_tokens", Value::number(800)),
                ("completion_tokens", Value::number(60)),
            ]),
        );
    }
    let fixture = HttpFixture::new(vec![(200, reply)]);
    let directory = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.set_effort(crate::effort::Effort::High);
    let messages = [Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Summarize this work")),
    ])];
    agent
        .complete_retry(&messages, false, Some(256), &mut |_| Ok(()))
        .unwrap();
    assert_eq!(agent.effort(), crate::effort::Effort::High);
    let request = fixture.finish().remove(0);
    assert_eq!(
        request
            .body
            .get("reasoning")
            .and_then(|reasoning| reasoning.get("effort"))
            .and_then(Value::as_str),
        Some("medium")
    );
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
        Some("context_compaction")
    );
    assert_eq!(timing.get("effort").and_then(Value::as_str), Some("medium"));
    assert_eq!(
        timing.get("input_tokens").and_then(Value::as_usize),
        Some(800)
    );
    assert_eq!(
        timing.get("output_tokens").and_then(Value::as_usize),
        Some(60)
    );
    assert_eq!(agent.context_status().tokens, None);
}

#[test]
fn background_title_usage_is_saved_without_changing_the_conversation_effort() {
    use super::backend_tests::fixture::{Account, Reply};
    let directory = Directory::new();
    let home = Directory::new();
    let account = Account::new("alpha", vec![Reply::text("Index repair")]);
    let mut agent = Agent::new(account.client(), Tools::new(directory.path()).unwrap());
    agent.set_effort(crate::effort::Effort::High);
    agent.enable_sessions(home.path()).unwrap();
    agent.name_sessions();
    agent.prepare_turn("Repair the index").unwrap();
    let handle = agent.sessions().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !handle.store().read(&handle.id()).is_ok_and(|saved| {
        saved
            .events
            .iter()
            .any(|event| event.get("operation").and_then(Value::as_str) == Some("session_title"))
    }) && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(handle.titled());
    assert_eq!(agent.effort(), crate::effort::Effort::High);
    let saved = handle.store().read(&handle.id()).unwrap();
    let title_events = saved
        .events
        .iter()
        .filter(|event| event.get("operation").and_then(Value::as_str) == Some("session_title"))
        .collect::<Vec<_>>();
    assert_eq!(title_events.len(), 1);
    let event = title_events[0];
    assert_eq!(
        event.get("status").and_then(Value::as_str),
        Some("complete")
    );
    assert_eq!(event.get("effort").and_then(Value::as_str), Some("low"));
    assert_eq!(
        event.get("input_tokens").and_then(Value::as_usize),
        Some(73)
    );
    assert_eq!(
        event.get("output_tokens").and_then(Value::as_usize),
        Some(4)
    );
    agent
        .run_turn("Repair the index", &mut |_| Ok(()))
        .unwrap_err();
    assert!(handle.store().read(&handle.id()).is_ok());
}
