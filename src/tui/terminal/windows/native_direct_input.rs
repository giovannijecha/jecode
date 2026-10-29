//! Native ConPTY exercise of the Rust terminal panel and direct PTY routing.
use super::*;
use crate::json::Value;

#[test]
#[ignore = "requires Windows ConPTY and PowerShell for native direct input"]
fn native_windows_direct_input() {
    let directory = Directory::new();
    let script = format!(
        "Add-Type -TypeDefinition @'\n{}\n'@\n[JecodeTestPty]::Run('{}', '{}', 'direct-input')\n",
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

pub(super) fn fixture(path: &std::path::Path, bash: &std::path::Path) {
    let script = "$first = Read-Host -Prompt ('NATIVE_' + 'PASSWORD_READY') -AsSecureString; $ptr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($first); try { $line = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($ptr); if (!$line.EndsWith([char]::ConvertFromUtf32(0x1F642))) { throw 'Unicode input lost' } } finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($ptr) }; Write-Output ('NATIVE_' + 'PTY_DONE'); $second = Read-Host -Prompt ('NATIVE_' + 'SECOND_READY') -AsSecureString; Write-Output ('NATIVE_' + 'PTY_END')";
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "native-pty",
                    "exec",
                    Value::object([
                        ("executable", Value::string("powershell.exe")),
                        (
                            "args",
                            Value::Array(
                                ["-NoLogo", "-NoProfile", "-Command", script]
                                    .into_iter()
                                    .map(Value::string)
                                    .collect(),
                            ),
                        ),
                        ("tty", Value::Bool(true)),
                        ("wait_ms", Value::number(0)),
                    ]),
                )],
            ),
        ),
        (200, completion("Native terminal started.", vec![])),
    ]);
    let mut settings =
        Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    // Interactive terminals require explicit Full authority on this host.
    settings.default_permissions = crate::permissions::Level::Full;
    let config = SessionConfig {
        store: Store::new(path.join("home")),
        settings,
        bash: bash.to_path_buf(),
    };
    config.store.save(&config.settings).unwrap();
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(path).unwrap(),
    );
    agent
        .permissions()
        .set_default(config.settings.default_permissions);
    crate::tui::launch(agent, config, None, Default::default(), None).unwrap();
    let sessions = crate::sessions::Store::new(path.join("home"), path)
        .unwrap()
        .list()
        .unwrap()
        .sessions;
    assert_eq!(sessions.len(), 1);
    let saved = crate::sessions::Store::new(path.join("home"), path)
        .unwrap()
        .fixture_load(&sessions[0].id)
        .unwrap();
    let archive = format!("{:?}{:?}{:?}", saved.input, saved.messages, saved.events);
    assert!(!archive.contains("NATIVE_SECRET_42"));
    assert_eq!(saved.input.draft.text, "kept draft");
    assert_eq!(fixture.finish().len(), 2);
    println!("NATIVE_DIRECT_INPUT_RESTORED");
}
