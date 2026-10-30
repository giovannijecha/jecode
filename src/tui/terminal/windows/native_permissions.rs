//! Real Windows renderer and keyboard path for conversation permissions.
use super::*;
use crate::{json::Value, permissions::Level};

#[test]
#[ignore = "requires Windows ConPTY and PowerShell for native permission input"]
fn native_windows_permissions() {
    let directory = Directory::new();
    let script = format!(
        "Add-Type -TypeDefinition @'\n{}\n'@\n[JecodeTestPty]::Run('{}', '{}', 'permissions')\n",
        native_pty_source(),
        std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .replace('\'', "''"),
        directory.path().to_string_lossy().replace('\'', "''"),
    );
    let result = process::run_observed(
        Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &script,
            ])
            .env("TEMP", directory.path())
            .env("TMP", directory.path())
            .env_remove("OPENROUTER_API_KEY"),
        None,
        Duration::from_secs(60),
        64 * 1024,
        &Cancellation::default(),
        None,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(
        result.exit_code,
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&result.stderr),
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(!result.timed_out);
}

pub(super) fn fixture(path: &std::path::Path) {
    let outside = Directory::new();
    let file = outside.path().join("outside-note.txt");
    std::fs::write(&file, "native outside fixture\n").unwrap();
    let read = |id| {
        tool_call(
            id,
            "read",
            Value::object([("path", Value::string(file.to_string_lossy()))]),
        )
    };
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![read("native-read-1")])),
        (200, completion("Native first read complete.", vec![])),
        (200, completion("", vec![read("native-read-2")])),
        (200, completion("Native second read complete.", vec![])),
    ]);
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let config = SessionConfig {
        store: Store::new(path.join("home")),
        settings,
    };
    config.store.save(&config.settings).unwrap();
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(path).unwrap(),
    );
    crate::tui::launch(agent, config, None, Default::default(), None).unwrap();
    assert_eq!(
        Store::new(path.join("home"))
            .load()
            .unwrap()
            .unwrap()
            .default_permissions,
        Level::Full
    );
    let store = crate::sessions::Store::new(path.join("home"), path).unwrap();
    let sessions = store.list().unwrap().sessions;
    assert_eq!(sessions.len(), 1);
    let saved = store.fixture_load(&sessions[0].id).unwrap();
    assert_eq!(
        crate::permissions::replay(&saved.events),
        (Some(Level::Workspace), vec![])
    );
    assert_eq!(saved.input.draft.text, "kept draft");
    assert!(
        saved.events.iter().any(|event| {
            event.get("type").and_then(Value::as_str) == Some("permission_revoke")
        })
    );
    let tool_result = |id| {
        let content = saved
            .messages
            .iter()
            .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some(id))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .unwrap();
        crate::json::parse(content).unwrap()
    };
    assert!(
        tool_result("native-read-1")
            .encode()
            .contains("native outside fixture")
    );
    assert_eq!(
        tool_result("native-read-2")
            .get("outcome")
            .and_then(Value::as_str),
        Some("denied")
    );
    assert_eq!(fixture.finish().len(), 4);
}
