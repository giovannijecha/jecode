use super::*;
use crate::checkpoints::session::Session;
use crate::test_support::Directory;
use std::fs;

#[test]
fn recovery_instructions_describe_per_call_scope_without_history_events() {
    let home = Directory::new();
    let root = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:9".into()),
        Tools::new(root.path()).unwrap(),
    );
    agent.use_checkpoints(Session::offline(home.path(), root.path()));
    let instructions = agent.checkpoint_instructions();
    assert!(instructions.contains("24 hours"));
    assert!(instructions.contains("inspect"));
    assert!(!instructions.contains("recovery group"));
    assert!(agent.events.lock().unwrap().is_empty());
    agent.deliver_checkpoint_notices();
    assert!(agent.events.lock().unwrap().is_empty());
}

#[test]
fn restoring_jecode_instructions_requests_a_single_reload() {
    let home = Directory::new();
    let root = Directory::new();
    let file = root.path().join("JECODE.md");
    fs::write(&file, "Use tabs.\n").unwrap();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:9".into()),
        Tools::new(root.path()).unwrap(),
    );
    agent.use_checkpoints(Session::offline(home.path(), root.path()));
    agent.refresh_project_instructions().unwrap();
    fs::write(&file, "Use spaces.\n").unwrap();
    agent.tools.checkpoints().unwrap().restored_instructions();
    agent.deliver_checkpoint_notices();
    assert!(agent.project_instructions.contains("Use spaces."));
    assert!(agent.events.lock().unwrap().is_empty());
    assert!(
        !agent
            .tools
            .checkpoints()
            .unwrap()
            .take_restored_instructions()
    );
}
