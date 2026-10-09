use super::*;
use crate::test_support::{Directory, HttpFixture, completion};

#[test]
fn resumed_requests_use_current_environment_and_keep_original_history() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("Previous answer", vec![])),
        (200, completion("Done", vec![])),
    ]);
    let make_agent = || {
        let mut agent = Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        );
        agent.enable_sessions(home.path()).unwrap();
        agent
    };
    let original = Value::object([
        ("role", Value::string("system")),
        ("content", Value::string("Legacy harness instructions.")),
    ]);
    let mut first = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    first.messages.lock().unwrap()[0] = original.clone();
    first.enable_sessions(home.path()).unwrap();
    first.run_turn("Original request", &mut |_| Ok(())).unwrap();
    first.save_session().unwrap();
    let id = first.sessions().unwrap().id();
    drop(first);

    let mut resumed = make_agent();
    resumed.resume(&id).unwrap();
    resumed.context.reset_usage();
    resumed.prepare_turn("Continue").unwrap();
    let projection = resumed.transport_messages();
    assert!(
        projection[0]
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .starts_with(
                resumed
                    .system_message()
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap()
            )
    );
    assert!(
        resumed.context_estimate(&resumed.messages.lock().unwrap())
            >= resumed
                .system_message()
                .encode()
                .len()
                .saturating_add(crate::tools::definitions().encode().len())
    );
    resumed.run_turn("Continue", &mut |_| Ok(())).unwrap();
    assert_eq!(resumed.messages.lock().unwrap()[0], original);
    assert_eq!(resumed.sessions().unwrap().snapshot().messages[0], original);
    let requests = fixture.finish();
    let sent = &requests[1]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap()[0];
    assert_eq!(sent, &projection[0]);
    assert!(sent.encode().contains("Environment:"));
    assert!(!sent.encode().contains("Legacy harness instructions"));
}
