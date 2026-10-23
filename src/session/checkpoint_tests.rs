use super::*;
use crate::{
    checkpoints::session::Session,
    config::{Settings, Store},
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture},
    tools::Tools,
};
use std::{fs, io::Cursor};

#[test]
fn plain_checkpoint_commands_restore_without_a_provider_request() {
    let home = Directory::new();
    let project = Directory::new();
    fs::write(project.path().join("a.txt"), "one\n").unwrap();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let recovery = Session::offline(home.path(), project.path());
    recovery.begin("fixture");
    let reference = recovery
        .store()
        .unwrap()
        .capture(
            "write",
            "fixture",
            &["a.txt".into()],
            &crate::cancel::Cancellation::default(),
        )
        .unwrap()
        .reference
        .unwrap();
    agent.use_checkpoints(recovery);
    let mut config = SessionConfig {
        store: Store::new(home.path().join("config.json")),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    };
    fs::write(project.path().join("a.txt"), "two\n").unwrap();

    let mut input = Cursor::new(format!(
        "/checkpoints list\n/checkpoints show {reference} a.txt\n/checkpoints restore {reference} a.txt --expected latest\n/checkpoints explode\n/exit\n"
    ));
    let mut output = Vec::new();
    let mut status = Vec::new();
    chat(
        &mut agent,
        &mut config,
        &mut input,
        &mut output,
        &mut status,
        Vec::new(),
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    let status = String::from_utf8(status).unwrap();
    assert!(output.contains("Checkpoints"), "{output}");
    assert!(
        output.contains("one"),
        "the recorded file is printed: {output}"
    );
    assert!(output.contains("restored"), "{output}");
    assert!(!output.contains(r"\\?\"), "{output}");
    assert!(status.contains("explode"), "{status}");
    assert_eq!(
        fs::read_to_string(project.path().join("a.txt")).unwrap(),
        "one\n"
    );
    assert!(fixture.finish().is_empty());
}
