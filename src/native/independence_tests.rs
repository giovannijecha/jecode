use crate::agent::Agent;
use crate::backend::openrouter::OpenRouter;
use crate::json::Value;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};
use crate::tools::Tools;
use std::process::{Command, Stdio};

const FIXTURE: &str = "native::independence_tests::turn_fixture";

#[test]
fn turn_fixture() {
    let Some(project) = std::env::var_os("JECODE_NATIVE_TURN_FIXTURE") else {
        return;
    };
    let project = std::path::PathBuf::from(project);
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "owned-shell",
                    "bash",
                    Value::object([(
                        "command",
                        Value::string(
                            "mkdir -p nested; printf ready > nested/value.txt; cat nested/value.txt",
                        ),
                    )]),
                )],
            ),
        ),
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "owned-read",
                    "read",
                    Value::object([("path", Value::string("nested/value.txt"))]),
                )],
            ),
        ),
        (200, completion("Finished", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&project).unwrap(),
    );
    #[cfg(not(windows))]
    agent
        .permissions()
        .set_level(crate::permissions::Level::Full)
        .unwrap();
    agent
        .run_turn("Create and inspect the fixture", &mut |_| Ok(()))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(project.join("nested/value.txt")).unwrap(),
        "ready"
    );
    let archive = agent.archive();
    let messages = archive.messages.lock().unwrap();
    for id in ["owned-shell", "owned-read"] {
        let message = messages
            .iter()
            .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some(id))
            .unwrap();
        let result =
            crate::json::parse(message.get("content").and_then(Value::as_str).unwrap()).unwrap();
        assert!(result.get("error").is_none(), "{result:?}");
        assert!(result.encode().contains("ready"), "{result:?}");
    }
    drop(messages);
    assert_eq!(fixture.finish().len(), 3);
}

#[test]
fn a_complete_turn_needs_no_optional_application() {
    let project = Directory::new();
    let home = Directory::new();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", FIXTURE, "--nocapture"])
        .env("JECODE_NATIVE_TURN_FIXTURE", project.path())
        .env("JECODE_HOME", home.path())
        .env("PATH", "")
        .env("JECODE_BASH", "missing-optional-bash")
        .env("JECODE_CURL", "missing-optional-curl")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(project.path().join("nested/value.txt")).unwrap(),
        "ready"
    );
    assert!(!project.path().join(".git").exists());
}
