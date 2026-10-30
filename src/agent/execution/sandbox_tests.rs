use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion, sandbox as probe, tool_call},
    tools::Tools,
};

fn result(agent: &Agent, id: &str) -> Value {
    let archive = agent.archive();
    let messages = archive.messages.lock().unwrap();
    let text = messages
        .iter()
        .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some(id))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .unwrap();
    json::parse(text).unwrap()
}

#[test]
fn workspace_agent_launches_direct_exec_through_sandbox() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "sandbox-hostname",
                    "exec",
                    Value::object([("executable", Value::string("hostname.exe"))]),
                )],
            ),
        ),
        (200, completion("Direct command finished", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent
        .run_turn("Run a direct command", &mut |_| Ok(()))
        .unwrap();
    let result = result(&agent, "sandbox-hostname");
    assert_eq!(
        result.get("exit_code"),
        Some(&Value::number(0)),
        "{result:?}"
    );
    assert!(
        result
            .get("stdout")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty())
    );
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn workspace_agent_launches_native_probe_through_sandbox() {
    let project = Directory::new();
    let home = Directory::new();
    let executable = probe::environment_executable(project.path());
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "sandbox-native-probe",
                    "exec",
                    Value::object([
                        ("executable", Value::string(executable.to_string_lossy())),
                        ("args", probe::environment_args()),
                        (
                            "env",
                            Value::object([("JECODE_TEST_ENV", Value::string("sentinel"))]),
                        ),
                    ]),
                )],
            ),
        ),
        (200, completion("Native command finished", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent
        .run_turn("Run a native command", &mut |_| Ok(()))
        .unwrap();
    let result = result(&agent, "sandbox-native-probe");
    assert_eq!(
        result.get("exit_code"),
        Some(&Value::number(0)),
        "{result:?}"
    );
    assert!(
        result
            .get("stdout")
            .and_then(Value::as_str)
            .is_some_and(|stdout| stdout.contains("WORKSPACE_NATIVE:sentinel:")),
        "{result:?}"
    );
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
#[ignore = "PSEC cannot initialize PowerShell in a Windows service session; run interactively"]
fn workspace_powershell_uses_installed_rust_tools_and_git_status() {
    let project = Directory::new();
    let home = Directory::new();
    let initialized = std::process::Command::new("git")
        .arg("init")
        .arg(project.path())
        .output()
        .unwrap();
    assert!(
        initialized.status.success(),
        "{}",
        String::from_utf8_lossy(&initialized.stderr)
    );
    let script = "cargo --version; if ($LASTEXITCODE -ne 0) { exit 21 }; rustc --version; if ($LASTEXITCODE -ne 0) { exit 22 }; git status --porcelain; if ($LASTEXITCODE -ne 0) { exit 23 }; Write-Output GIT_STATUS_OK";
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "sandbox-powershell-tools",
                    "exec",
                    Value::object([
                        ("executable", Value::string("powershell.exe")),
                        (
                            "args",
                            Value::Array(
                                ["-NoProfile", "-Command", script]
                                    .into_iter()
                                    .map(Value::string)
                                    .collect(),
                            ),
                        ),
                    ]),
                )],
            ),
        ),
        (200, completion("Tool check finished", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent
        .run_turn("Check installed tools", &mut |_| Ok(()))
        .unwrap();
    let result = result(&agent, "sandbox-powershell-tools");
    assert_eq!(
        result.get("exit_code"),
        Some(&Value::number(0)),
        "{result:?}"
    );
    let output = result.get("stdout").and_then(Value::as_str).unwrap_or("");
    for expected in ["cargo", "rustc", "GIT_STATUS_OK"] {
        assert!(output.contains(expected), "{result:?}");
    }
    assert_eq!(fixture.finish().len(), 2);
}
