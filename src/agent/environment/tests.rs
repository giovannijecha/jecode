use super::*;
use crate::json;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};

fn state(message: &Value, label: &str) -> Value {
    let text = message.get("content").unwrap().as_str().unwrap();
    let (_, tail) = text.split_once(label).unwrap();
    json::parse(tail.lines().next().unwrap()).unwrap()
}

#[test]
fn a_new_request_exposes_stale_memory_without_selecting_its_next_action() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = Agent::new(
        crate::openrouter::OpenRouter::fixture(fixture.endpoint.clone()),
        crate::tools::Tools::new(directory.path()).unwrap(),
    );
    agent.messages.lock().unwrap().extend([
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Build index")),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("Index work started")),
        ]),
    ]);
    let mut memory = json::parse(&crate::context::memory::fixture("Build index")).unwrap();
    if let Value::Object(fields) = &mut memory {
        fields.insert("reviewed_request_history".into(), Value::number(1));
        fields.insert("next_action".into(), Value::string("Write index.rs"));
    }
    agent.context.from = 3;
    agent.context.summary = memory.encode();
    agent.prepare_turn("Cancel index; audit only").unwrap();
    let original = agent.messages.lock().unwrap().clone();
    let previous_context = agent.context.value();
    let sent = agent.transport_messages();
    let request = state(&sent[0], "Native request state:\n");
    let context = state(&sent[1], "Native context state:\n");
    assert_eq!(
        request.get("latest_user_request"),
        Some(&Value::string("history:3"))
    );
    assert_eq!(
        context.get("latest_user_request"),
        request.get("latest_user_request")
    );
    assert_eq!(
        context.get("memory_reviewed_request"),
        Some(&Value::string("history:1"))
    );
    assert_eq!(
        context.get("memory_proves_execution"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        context.get("memory").unwrap().get("next_action"),
        memory.get("next_action")
    );
    let environment = state(&sent[0], "Environment:\n");
    assert_eq!(
        environment.get("working_directory"),
        Some(&Value::string(agent.tools.root().to_string_lossy()))
    );
    assert!(!sent[0].encode().contains("saved next action"));
    assert!(!sent[0].encode().contains("Write index.rs"));
    assert!(
        sent.iter()
            .any(|message| message.get("content").and_then(Value::as_str)
                == Some("Cancel index; audit only"))
    );
    assert_eq!(*agent.messages.lock().unwrap(), original);
    assert_eq!(agent.context.value(), previous_context);
    assert!(fixture.finish().is_empty());
}

#[test]
fn completion_review_exposes_failed_check_and_withheld_response_then_delivers_the_replacement() {
    let directory = Directory::new();
    let command = |id, text| {
        tool_call(
            id,
            "bash",
            Value::object([
                ("command", Value::string(text)),
                ("check", Value::Bool(true)),
            ]),
        )
    };
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![command("fail", "exit 7")])),
        (200, completion("Everything passed.", vec![])),
        (
            200,
            completion("", vec![command("verify", "printf verified")]),
        ),
        (
            200,
            completion("Verified after correcting the failed check.", vec![]),
        ),
    ]);
    let mut agent = Agent::new(
        crate::openrouter::OpenRouter::fixture(fixture.endpoint.clone()),
        crate::tools::Tools::new(directory.path()).unwrap(),
    );
    let mut delivered = Vec::new();
    agent
        .run_turn("Verify and report", &mut |event| {
            if let crate::events::Event::Message { text } = event {
                delivered.push(text);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(delivered, ["Verified after correcting the failed check."]);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 4);
    let messages = requests[2]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    let review = state(&messages[0], "Native completion state:\n");
    assert_eq!(review.get("status"), Some(&Value::string("reviewing")));
    assert_eq!(
        review.get("candidate_response"),
        Some(&Value::string("history:4"))
    );
    assert_eq!(review.get("candidate_delivered"), Some(&Value::Bool(false)));
    assert_eq!(review.get("native_blocker"), Some(&Value::Null));
    let context = state(&messages[1], "Native context state:\n");
    let check = &context
        .get("execution_facts")
        .unwrap()
        .get("checks")
        .unwrap()
        .as_array()
        .unwrap()[0];
    assert_eq!(check.get("check_status"), Some(&Value::string("failed")));
    assert_eq!(
        check.get("source").unwrap().get("request_history"),
        Some(&Value::string("history:1"))
    );
    let messages = requests[3]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    let review = state(&messages[0], "Native completion state:\n");
    assert_eq!(
        review.get("candidate_response"),
        Some(&Value::string("history:4"))
    );
    assert_eq!(review.get("candidate_delivered"), Some(&Value::Bool(false)));
}
