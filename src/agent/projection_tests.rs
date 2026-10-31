use super::*;
use crate::{permissions::Level, test_support::Directory};

#[test]
fn runtime_changes_preserve_the_durable_prefix_and_the_original_history() {
    let directory = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.permissions.set_level(Level::Workspace).unwrap();
    agent
        .prepare_turn("Inspect the project and retain the original evidence.")
        .unwrap();
    let archived = agent.messages.lock().unwrap().clone();
    let before = agent.transport_messages();
    agent.permissions.set_level(Level::Full).unwrap();
    let after = agent.transport_messages();
    assert_eq!(before[..archived.len()], after[..archived.len()]);
    assert_eq!(before[0], agent.current_system_message());
    assert_eq!(*agent.messages.lock().unwrap(), archived);
    assert_ne!(before.last(), after.last());
    assert_eq!(
        after.last().unwrap().get("jecode_request_state"),
        Some(&Value::Bool(true))
    );
    assert!(after.last().unwrap().encode().contains("Permissions: Full"));
    assert!(!after[0].encode().contains("Permissions: Full"));
}

#[test]
fn runtime_suffix_is_accounted_for_without_creating_a_saved_user_request() {
    let directory = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.prepare_turn("Continue the existing task.").unwrap();
    let request = agent.transport_request();
    let expected = (crate::context::bytes_for(&request.messages, agent.client.inputs())
        + crate::tools::encoded_size())
    .div_ceil(4);
    assert_eq!(request.estimated_tokens, expected);
    assert!(
        agent
            .messages
            .lock()
            .unwrap()
            .iter()
            .all(|message| message.get("jecode_request_state").is_none())
    );
    assert!(
        request
            .messages
            .last()
            .unwrap()
            .encode()
            .contains("not a new user task")
    );
}
