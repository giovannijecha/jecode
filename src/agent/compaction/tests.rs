use super::*;
use crate::test_support::{Directory, HttpFixture};

#[test]
fn recent_retention_counts_known_reasoning_instead_of_only_visible_bytes() {
    let directory = Directory::new();
    let client = crate::backend::openrouter::OpenRouter::fixture("http://127.0.0.1:1".into());
    let agent = Agent::new(client, crate::tools::Tools::new(directory.path()).unwrap());
    let origin = agent.identity();
    let mut source = agent.messages.lock().unwrap().clone();
    source.push(Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Inspect")),
    ]));
    for _ in 0..3 {
        let mut response = Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("Short visible answer")),
        ]);
        crate::conversation::stamp(&mut response, &origin);
        crate::conversation::record_output_tokens(&mut response, Some(50_000));
        source.push(response);
    }
    assert_eq!(agent.recent_boundary(&source, 80_000), source.len() - 1);
}

#[test]
fn resume_keeps_saved_model_calibration_but_discards_direct_request_measurements() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let new_agent = || {
        let client = crate::backend::openrouter::OpenRouter::fixture(fixture.endpoint.clone());
        Agent::new(client, crate::tools::Tools::new(directory.path()).unwrap())
    };
    let mut first = new_agent();
    first.enable_sessions(home.path()).unwrap();
    first.prepare_turn("Continue the same work").unwrap();
    first.context.observe(
        Some(&crate::backend::Usage {
            input_tokens: Some(1000),
            ..Default::default()
        }),
        2,
    );
    first.context.calibrate(6000);
    first.save_session().unwrap();
    let id = first.sessions().unwrap().id();
    drop(first);
    let mut resumed = new_agent();
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.context.calibration, Some((1000, 6000)));
    assert_eq!(resumed.context.input_tokens, None);
    assert_eq!(resumed.context.measured_end, 0);
    resumed.set_model("fixture/other".into()).unwrap();
    assert_eq!(resumed.context.calibration, None);
    assert!(fixture.finish().is_empty());
}

#[test]
fn a_latest_request_larger_than_the_recent_tail_is_shown_as_a_marked_excerpt_with_its_reference() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(
        (0..8)
            .map(|_| {
                (
                    200,
                    crate::test_support::completion(
                        "Parser inspected; the new request is pending.",
                        vec![],
                    ),
                )
            })
            .collect(),
    );
    let mut agent = Agent::new(
        crate::backend::openrouter::OpenRouter::fixture(fixture.endpoint.clone()),
        crate::tools::Tools::new(directory.path()).unwrap(),
    );
    let latest = format!("FIRST LINE {}LAST LINE", "latest requirement ".repeat(3000));
    let message = |role: &str, text: &str| {
        Value::object([
            ("role", Value::string(role)),
            ("content", Value::string(text)),
        ])
    };
    agent.messages.lock().unwrap().extend([
        message("user", "Inspect the parser."),
        message("assistant", &"Parser inspected. ".repeat(200)),
        message("user", &latest),
    ]);
    let limits = crate::backend::Limits {
        context: agent.fixture_capacity(latest.len(), 80),
        output: Some(4096),
    };
    assert!(
        agent
            .compact_if_needed(Some(limits), true, &mut |_| Ok(()))
            .unwrap()
    );
    let original = agent.messages.lock().unwrap().clone();
    // Nothing fits the recent tail, so the whole conversation is summarized.
    assert_eq!(agent.context.from, original.len());
    agent.context.validate(&original).unwrap();
    let view = Value::Array(agent.transport_messages()).encode();
    for expected in [
        "Last user request in this checkpoint: history:3",
        "[... context excerpt; full original remains in history ...]",
        "FIRST LINE",
        "LAST LINE",
        "Parser inspected; the new request is pending.",
    ] {
        assert!(view.contains(expected), "{expected}");
    }
    assert!(view.len() < latest.len());
    assert_eq!(
        original[3].get("content").and_then(Value::as_str),
        Some(latest.as_str())
    );
    // Bounded portions; the spare fixture responses stay unused.
    assert!(fixture.finish_received().len() < 8);
}
