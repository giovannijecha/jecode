use super::*;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};

#[test]
fn an_ordinary_notice_cannot_claim_native_report_delivery() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.prepare_turn("Report").unwrap();
    agent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string("Unaccepted text")),
    ]));
    agent.record_local_details(
        "Report delivery",
        "Ordinary notice",
        "notice",
        &[
            ("request_history".into(), "1".into()),
            ("response_history".into(), "2".into()),
        ],
    );
    assert!(agent.delivered_report_hint().is_empty());
    assert!(fixture.finish().is_empty());
}

#[test]
fn only_the_accepted_final_response_has_a_delivery_receipt() {
    let directory = Directory::new();
    std::fs::write(directory.path().join("source"), "Original").unwrap();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "read",
                    "read",
                    Value::object([("path", Value::string("source"))]),
                )],
            ),
        ),
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "change",
                    "bash",
                    Value::object([
                        ("command", Value::string("printf Changed > source")),
                        ("check", Value::Bool(false)),
                    ]),
                )],
            ),
        ),
        (200, completion("Provisional response", vec![])),
        (200, completion("Accepted final response", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent
        .run_turn("Change source and report", &mut |_| Ok(()))
        .unwrap();
    let original = agent.messages.lock().unwrap().clone();
    assert_eq!(
        original[6].get("content").and_then(Value::as_str),
        Some("Provisional response")
    );
    assert_eq!(
        original[7].get("content").and_then(Value::as_str),
        Some("Accepted final response")
    );
    let hint = agent.delivered_report_hint();
    assert!(hint.contains("response at history:7"));
    assert!(hint.contains("request history:1"));
    assert!(!hint.contains("response at history:6"));
    assert!(hint.contains("not correctness, passed checks or acceptance coverage"));
    assert_eq!(
        agent
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| {
                event.get("command").and_then(Value::as_str) == Some("Report delivery")
            })
            .count(),
        1
    );
    assert_eq!(*agent.messages.lock().unwrap(), original);
    assert_eq!(fixture.finish().len(), 4);
}

#[test]
fn a_failed_response_delivery_cannot_be_recorded_as_delivered() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Undelivered response", vec![]))]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let error = agent
        .run_turn("Report", &mut |event| {
            if matches!(event, Event::Message { .. }) {
                Err("Delivery rejected".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error, "Delivery rejected");
    assert!(agent.delivered_report_hint().is_empty());
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn resumed_summary_receives_a_native_receipt_without_rewriting_original_messages() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("Delivered response", vec![])),
        (
            200,
            completion(
                &crate::context::memory::fixture("Continue from delivered work"),
                vec![],
            ),
        ),
    ]);
    let make_agent = || {
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        )
    };
    let mut agent = make_agent();
    agent.enable_sessions(home.path()).unwrap();
    agent.run_turn("Report", &mut |_| Ok(())).unwrap();
    let id = agent.sessions().unwrap().id();
    let original = agent.messages.lock().unwrap().clone();
    drop(agent);
    let mut resumed = make_agent();
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert!(resumed.sessions().unwrap().snapshot().records().iter().all(|record| {
        !matches!(record, crate::sessions::Record::Local { command, .. } if command == "Report delivery")
    }));
    assert!(
        resumed
            .delivered_report_hint()
            .contains("response at history:2")
    );
    let records = original
        .iter()
        .enumerate()
        .skip(1)
        .map(|(index, message)| (index, message.encode()))
        .collect::<Vec<_>>();
    resumed
        .summarize(
            &records,
            "Report",
            crate::openrouter::Limits {
                context: 24000,
                output: None,
            },
            4096,
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(*resumed.messages.lock().unwrap(), original);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    let summary = requests[1].body.encode();
    assert!(summary.contains("Native report delivery"));
    assert!(summary.contains("later user requests can require another response"));
    assert!(summary.contains("not correctness, passed checks or acceptance coverage"));
}
