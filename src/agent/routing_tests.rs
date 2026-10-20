use super::*;
use crate::{
    openrouter::Routing,
    test_support::{Directory, HttpFixture, completion},
};
use std::collections::BTreeMap;

const PIN: &str = "two/fp8";

/// Three measured endpoints: medians of 40 tok/s and 1 s make Auto prefer at
/// least 20 tok/s and at most 2 s. `two/fp8/eu` extends the pinned slug.
fn listing() -> Value {
    crate::json::parse(
        r#"{"data":{"endpoints":[
        {"tag":"one/fp8","status":0,"supported_parameters":["tools"],
        "perf_last_30m_by_workload":{"text_generation":{"latency":{"p50":800},"throughput":{"p50":60},"request_count":40}}},
        {"tag":"two/fp8","status":0,"supported_parameters":["tools"],
        "perf_last_30m_by_workload":{"text_generation":{"latency":{"p50":1000},"throughput":{"p50":40},"request_count":40}}},
        {"tag":"two/fp8/eu","status":0,"supported_parameters":["tools"]},
        {"tag":"three","status":0,"supported_parameters":["tools"],
        "perf_last_30m_by_workload":{"text_generation":{"latency":{"p50":1500},"throughput":{"p50":20},"request_count":40}}}
        ]}}"#,
    )
    .unwrap()
}

fn served(text: &str, provider: &str, cost: f64) -> Value {
    let Value::Object(mut fields) = completion(text, vec![]) else {
        unreachable!()
    };
    fields.insert("provider".into(), Value::string(provider));
    fields.insert(
        "usage".into(),
        Value::object([("cost", Value::number(cost))]),
    );
    Value::Object(fields)
}

fn error(status: u16, message: &str) -> (u16, Value) {
    (
        status,
        Value::object([(
            "error",
            Value::object([
                ("code", Value::number(status)),
                ("message", Value::string(message)),
            ]),
        )]),
    )
}

fn agent(directory: &Directory, endpoint: &str, routing: Routing) -> Agent {
    let mut agent = Agent::new(
        OpenRouter::fixture(endpoint.into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.set_routing(routing);
    agent
}

fn provider(request: &crate::test_support::Request) -> Option<String> {
    request.body.get("provider").map(Value::encode)
}

#[test]
fn auto_sends_the_conversation_id_and_preferences_and_lists_endpoints_once_per_window() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, listing()),
        (200, served("First", "One", 0.002)),
        (200, served("Second", "Three", 0.0031)),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint, Routing::Auto);
    agent.enable_sessions(home.path()).unwrap();
    agent.run_turn("First task", &mut |_| Ok(())).unwrap();
    assert_eq!(
        agent.charges().describe().unwrap(),
        "Served by One · $0.002 charged"
    );
    agent.run_turn("Second task", &mut |_| Ok(())).unwrap();
    // Each turn reports only what its own responses said.
    assert_eq!(
        agent.charges().describe().unwrap(),
        "Served by Three · $0.0031 charged"
    );
    let id = agent.sessions().unwrap().id();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].headers[0].starts_with("GET /models/fixture/model/endpoints "));
    for request in &requests[1..] {
        assert_eq!(
            request.body.get("session_id").and_then(Value::as_str),
            Some(id.as_str())
        );
        assert_eq!(
            provider(request).as_deref(),
            Some(r#"{"preferred_max_latency":2,"preferred_min_throughput":20}"#)
        );
    }
}

#[test]
fn a_pin_reaches_summary_requests_and_its_failures_say_how_to_change_it() {
    let directory = Directory::new();
    // Leave summary room after a rejection of the native prompt and tool contract.
    let prompt = "original objective ".repeat(1500);
    let fixture = HttpFixture::new(vec![
        (200, listing()),
        (
            400,
            Value::object([(
                "error",
                Value::object([
                    ("code", Value::string("context_length_exceeded")),
                    ("message", Value::string("Maximum context length exceeded")),
                ]),
            )]),
        ),
        (200, completion("Original objective retained.", vec![])),
        (200, completion("Finished.", vec![])),
        error(404, "No endpoints found matching your data policy"),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint, Routing::Endpoint(PIN.into()));
    agent.run_turn(&prompt, &mut |_| Ok(())).unwrap();
    let failure = agent.run_turn("Next task", &mut |_| Ok(())).unwrap_err();
    assert!(
        failure.contains(
            "Pinned endpoint two/fp8 has no fallbacks; use /routing to choose Auto or another endpoint."
        ),
        "{failure}"
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 5);
    // The summary request is the one without tools.
    assert!(requests[2].body.get("tools").is_none());
    for request in &requests[1..] {
        assert_eq!(
            provider(request).as_deref(),
            Some(r#"{"allow_fallbacks":false,"ignore":["two/fp8/eu"],"order":["two/fp8"]}"#)
        );
        assert!(request.body.get("session_id").is_none());
    }
}

#[test]
fn an_unavailable_listing_never_blocks_a_turn_and_is_retried_only_after_a_delay() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        error(503, "Catalog unavailable"),
        (200, completion("First", vec![])),
        (200, completion("Second", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint, Routing::Auto);
    agent.run_turn("First task", &mut |_| Ok(())).unwrap();
    agent.run_turn("Second task", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].headers[0].starts_with("GET "));
    // Without fresh statistics Auto leaves the choice to native routing.
    assert!(
        requests[1..]
            .iter()
            .all(|request| provider(request).is_none())
    );
}

#[test]
fn conversations_take_saved_preferences_while_resume_restores_their_own_routing() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, listing()),
        (200, completion("Pinned answer", vec![])),
        (200, completion("Auto answer", vec![])),
    ]);
    let pin = Routing::Endpoint(PIN.into());
    let other = Routing::Endpoint("other/tag".into());
    let preferences = BTreeMap::from([
        ("fixture/model".to_owned(), pin.clone()),
        ("fixture/other".to_owned(), other.clone()),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint, pin.clone());
    agent.set_routing_preferences(preferences);
    agent.enable_sessions(home.path()).unwrap();
    agent.run_turn("Pinned task", &mut |_| Ok(())).unwrap();
    let pinned = agent.sessions().unwrap().id();
    agent
        .start_new("fixture/model".into(), Effort::Default)
        .unwrap();
    assert_eq!(agent.routing(), &pin);
    // A conversation choice leaves the saved preference alone.
    agent.set_routing(Routing::Auto);
    agent.run_turn("Auto task", &mut |_| Ok(())).unwrap();
    let automatic = agent.sessions().unwrap().id();
    assert_eq!(agent.preferred_routing("fixture/model"), pin);

    agent.resume(&pinned).unwrap();
    assert_eq!(agent.routing(), &pin);
    agent.resume(&automatic).unwrap();
    assert_eq!(agent.routing(), &Routing::Auto);
    // A model switch never keeps another model's pin.
    agent.set_model("fixture/third".into()).unwrap();
    assert_eq!(agent.routing(), &Routing::Auto);
    agent.set_model("fixture/other".into()).unwrap();
    assert_eq!(agent.routing(), &other);
    let (_, current) = agent
        .delete_session(&automatic, "fixture/model".into(), Effort::Default)
        .unwrap();
    assert!(current);
    assert_eq!(agent.routing(), &pin);
    assert_eq!(agent.sessions().unwrap().snapshot().routing, pin);

    let requests = fixture.finish();
    // The same model keeps its fresh listing across conversations.
    assert_eq!(requests.len(), 3);
    assert_eq!(
        provider(&requests[1]).as_deref(),
        Some(r#"{"allow_fallbacks":false,"ignore":["two/fp8/eu"],"order":["two/fp8"]}"#)
    );
    assert_eq!(
        provider(&requests[2]).as_deref(),
        Some(r#"{"preferred_max_latency":2,"preferred_min_throughput":20}"#)
    );
    assert_eq!(
        requests[2].body.get("session_id").and_then(Value::as_str),
        Some(automatic.as_str())
    );
}

#[test]
fn routing_charges_include_billed_responses_rejected_as_incomplete() {
    let directory = Directory::new();
    let mut truncated = served("Partial output", "One", 0.004);
    let Value::Object(fields) = &mut truncated else {
        unreachable!()
    };
    let Value::Array(choices) = fields.get_mut("choices").unwrap() else {
        unreachable!()
    };
    if let Value::Object(choice) = &mut choices[0] {
        choice.insert("finish_reason".into(), Value::string("length"));
    }
    let fixture = HttpFixture::new(vec![(200, truncated)]);
    let mut agent = agent(&directory, &fixture.endpoint, Routing::Default);
    let error = agent.run_turn("A task", &mut |_| Ok(())).unwrap_err();
    assert!(error.contains("finish_reason: length"), "{error}");
    assert_eq!(agent.charges().cost, 0.004);
    assert_eq!(agent.charges().requests, 1);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn routing_last_turn_charges_do_not_follow_conversation_switches() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, served("First", "One", 0.002)),
        (200, served("Second", "Two", 0.003)),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint, Routing::Default);
    agent.enable_sessions(home.path()).unwrap();
    let first = agent.sessions().unwrap().id();
    agent.run_turn("First task", &mut |_| Ok(())).unwrap();
    assert!(agent.charges().describe().is_some());
    agent
        .start_new("fixture/model".into(), Effort::Default)
        .unwrap();
    assert_eq!(agent.charges().describe(), None);
    agent.run_turn("Second task", &mut |_| Ok(())).unwrap();
    assert!(agent.charges().describe().is_some());
    agent.resume(&first).unwrap();
    assert_eq!(agent.charges().describe(), None);
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn routing_pin_requires_metadata_before_sending_a_completion() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![error(503, "Catalog unavailable")]);
    let mut agent = agent(&directory, &fixture.endpoint, Routing::Endpoint(PIN.into()));
    // Stop the old implementation's retry if it attempts a completion after
    // the metadata server has closed, instead of rejecting the unverified pin.
    let cancellation = agent.cancellation.clone();
    let error = agent
        .run_turn("A task", &mut |event| {
            if matches!(event, crate::events::Event::Recovering { .. }) {
                cancellation.cancel();
            }
            Ok(())
        })
        .unwrap_err();
    assert!(error.contains("Cannot verify pinned endpoint"), "{error}");
    assert_eq!(fixture.finish().len(), 1);
}
