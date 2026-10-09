use super::*;
use crate::test_support::{Directory, HttpFixture};

#[test]
fn completion_review_shows_the_live_blocker_and_counts_only_new_state_growth() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.prepare_turn("Inspect and report").unwrap();
    agent
        .context
        .evidence
        .update_file_constraints(vec![Value::object([
            ("path", Value::string("acceptance.txt")),
            ("state", Value::string("preserved")),
            ("require_check", Value::Bool(true)),
        ])]);
    let problem = agent.context.evidence.protection_problem().unwrap();
    agent.context.observe(
        Some(&Value::object([("prompt_tokens", Value::number(1000))])),
        2,
    );
    agent.reviewing_completion = true;
    let review = agent.completion_review_state(&agent.messages.lock().unwrap());
    assert!(
        agent.transport_messages()[0]
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains(&problem)
    );
    assert_eq!(
        agent.context_estimate(&agent.messages.lock().unwrap()),
        1000 + review.len()
    );
    agent.measured_review_bytes = review.len();
    assert_eq!(
        agent.context_estimate(&agent.messages.lock().unwrap()),
        1000
    );
    agent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Additional requirement")),
    ]));
    let messages = agent.messages.lock().unwrap();
    assert_eq!(
        agent.context_estimate(&messages),
        1000 + crate::context::bytes(&messages[2..])
    );
    drop(messages);
    assert!(agent.context.evidence.protection_problem().is_some());
    assert!(fixture.finish().is_empty());
}
