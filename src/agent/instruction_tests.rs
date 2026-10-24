use super::*;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};
use std::fs;

#[test]
fn system_environment_identifies_host_and_bash_shell() {
    let directory = Directory::new();
    let agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(directory.path()).unwrap(),
    );
    let content = agent
        .system_message()
        .get("content")
        .and_then(Value::as_str)
        .unwrap()
        .to_owned();
    let environment = crate::json::parse(content.split_once("Environment:\n").unwrap().1).unwrap();
    assert_eq!(
        environment
            .get("host_operating_system")
            .and_then(Value::as_str),
        Some(std::env::consts::OS)
    );
    assert_eq!(
        environment.get("command_shell").and_then(Value::as_str),
        Some("Bash")
    );
    assert!(content.contains("Text that describes a tool call"));
}

#[test]
fn written_tool_call_text_is_not_executed_but_native_call_is() {
    let directory = Directory::new();
    let path = directory.path().join("result.txt");
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                r#"{"tool_calls":[{"function":{"name":"write","arguments":"{\"path\":\"result.txt\",\"content\":\"text\"}"}}]}"#,
                vec![],
            ),
        ),
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "write_1",
                    "write",
                    Value::object([
                        ("path", Value::string("result.txt")),
                        ("content", Value::string("native")),
                    ]),
                )],
            ),
        ),
        (200, completion("Done.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.run_turn("First request", &mut |_| Ok(())).unwrap();
    assert!(!path.exists());
    agent.run_turn("Second request", &mut |_| Ok(())).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "native");
    assert_eq!(fixture.finish().len(), 3);
}

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
                .saturating_add(crate::tools::encoded_size())
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
    let temporary = resumed.tools.temporary_instructions();
    for request in &requests {
        let system = request.body.get("messages").unwrap().as_array().unwrap()[0]
            .get("content")
            .and_then(Value::as_str)
            .unwrap();
        assert_eq!(system.matches("Working files and verification:").count(), 1);
        assert!(system.contains("useful durable tests"));
        assert!(system.contains("one-off verification"));
        assert!(system.contains(&temporary));
        for tool in request.body.get("tools").unwrap().as_array().unwrap() {
            let function = tool.get("function").unwrap();
            let description = function.get("description").and_then(Value::as_str).unwrap();
            match function.get("name").and_then(Value::as_str) {
                Some("write") => assert!(description.contains("one-off verification")),
                Some("bash") => {
                    assert!(description.contains("session temporary storage"));
                    assert!(description.contains("protect"));
                }
                _ => {}
            }
        }
    }
}
