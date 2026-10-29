use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    config::{Settings, Store},
    permissions::{Access, Decision, Kind, Level, Surface, Verdict},
    test_support::Directory,
    tools::Tools,
};
use std::io::Cursor;

fn setup(project: &Directory, home: &Directory) -> (Agent, SessionConfig) {
    let agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    let config = SessionConfig {
        store: Store::new(home.path().join("config.json")),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    };
    (agent, config)
}

#[test]
fn plain_permissions_selects_only_this_conversation_and_reports_invalid_level() {
    let project = Directory::new();
    let home = Directory::new();
    let (mut agent, mut config) = setup(&project, &home);
    let mut output = Vec::new();
    let mut status = Vec::new();
    chat(
        &mut agent,
        &mut config,
        &mut Cursor::new(
            "/help\n/permissions\n/permissions full\n/permissions typo\n/permissions read\n/permissions\n/new\n/permissions\n/exit\n",
        ),
        &mut output,
        &mut status,
        Vec::new(),
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    let status = String::from_utf8(status).unwrap();
    assert!(output.contains("/permissions [LEVEL]"), "{output}");
    assert!(output.contains("Permissions: Workspace → Full · this conversation"));
    assert!(output.contains("Permissions: Full → Read · this conversation"));
    assert!(output.contains("Read (current) · Read and analyze"));
    assert!(output.contains("Default for new conversations: Workspace"));
    assert!(status.contains("Usage: /permissions [read|workspace|full|remembered|revoke N]"));
    assert_eq!(agent.permissions().level(), Level::Workspace);
    assert_eq!(agent.permissions().default_level(), Level::Workspace);
    assert_eq!(config.settings.default_permissions, Level::Workspace);
    assert!(config.store.load().unwrap().is_none());
}

#[test]
fn plain_permissions_lists_and_revokes_remembered_access() {
    let project = Directory::new();
    let home = Directory::new();
    let (mut agent, mut config) = setup(&project, &home);
    agent.enable_sessions(home.path()).unwrap();
    let permissions = agent.permissions();
    let access = Access {
        kind: Kind::Reads,
        target: "outside/file.txt".into(),
        place: None,
        outside: true,
        base: None,
        identity: None,
    };
    permissions.set_surface(Surface::Panel);
    let request = permissions.begin("call-1", "read_file", &access).unwrap();
    permissions.decide(request.id, Decision::Remember).unwrap();
    permissions.set_surface(Surface::None);
    assert!(matches!(permissions.check(&access), Verdict::Allow));
    let mut output = Vec::new();
    let mut status = Vec::new();
    chat(
        &mut agent,
        &mut config,
        &mut Cursor::new(
            "/permissions remembered\n/permissions revoke 99\n/permissions read\n/permissions remembered\n/permissions revoke 1\n/permissions remembered\n/exit\n",
        ),
        &mut output,
        &mut status,
        Vec::new(),
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    let status = String::from_utf8(status).unwrap();
    assert!(output.contains("1 Reads · outside/file.txt"), "{output}");
    assert!(output.contains("Paused at Read."), "{output}");
    assert!(output.contains("Revoked grant 1 · Reads of outside/file.txt"));
    assert!(output.contains("Nothing remembered for this conversation."));
    assert!(status.contains("No remembered grant 99."));
    assert!(permissions.grants().is_empty());
    permissions.set_level(Level::Workspace).unwrap();
    assert!(matches!(permissions.check(&access), Verdict::Ask));
    assert_eq!(config.settings.default_permissions, Level::Workspace);
}
