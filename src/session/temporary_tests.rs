use super::*;
use crate::{
    config::{Settings, Store},
    json::Value,
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::{fs, io::Cursor};

#[test]
fn plain_chat_reports_and_cleans_temporary_files_without_a_provider_request_for_commands() {
    let home = Directory::new();
    let project = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "probe",
                    "write",
                    Value::object([
                        ("path", Value::string("tmp:probe.txt")),
                        ("content", Value::string("scratch")),
                    ]),
                )],
            ),
        ),
        (200, completion("Probe saved outside the project", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    let mut config = SessionConfig {
        store: Store::new(home.path().join("config.json")),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    };
    let mut input = Cursor::new(
        "/tmp\nMake a scratch probe\n/tmp\n/tmp clean\n/tmp\n/new\n/tmp\n/tmp nonsense\n/exit\n",
    );
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
    assert_eq!(output.matches("Session temporary files").count(), 4);
    assert!(output.contains("7 bytes"));
    assert!(output.contains("Cleared 1 temporary files"));
    assert!(output.contains("Kept until /tmp clean"));
    assert!(
        String::from_utf8(status)
            .unwrap()
            .contains("Usage: /tmp [clean]")
    );
    assert_eq!(agent.temporary_info().unwrap().files, 0);
    assert_eq!(fs::read_dir(project.path()).unwrap().count(), 0);
    assert_eq!(fixture.finish().len(), 2);
}
