use super::*;

#[test]
#[ignore = "requires Windows ConPTY for isolated application startup"]
fn native_windows_startup_without_credentials() {
    for scenario in [
        "startup-removed",
        "startup-fresh",
        "startup-fallback",
        "startup-key-removed",
    ] {
        check(scenario);
    }
}

fn check(scenario: &str) {
    let directory = Directory::new();
    let script = format!(
        "Add-Type -TypeDefinition @'\n{}\n'@\n[JecodeTestPty]::Run('{}', '{}', '{}')\n",
        include_str!("native_pty.cs"),
        std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .replace('\'', "''"),
        directory.path().to_string_lossy().replace('\'', "''"),
        scenario,
    );
    let result = process::run_observed(
        Command::new("powershell.exe").args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ]),
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

pub(super) fn fixture(scenario: &str) {
    let path = std::path::PathBuf::from(std::env::var_os("JECODE_NATIVE_DIR").unwrap());
    let store = Store::new(path);
    if scenario != "startup-fresh" {
        let mut settings =
            Settings::selected("openai", String::new(), "fixture/model".into()).unwrap();
        if scenario == "startup-fallback" {
            settings
                .credentials
                .insert("openrouter".into(), "isolated-fixture-key".into());
        } else if scenario == "startup-key-removed" {
            settings
                .select_backend("openrouter", String::new())
                .unwrap();
        }
        store.save(&settings).unwrap();
    }
    crate::bootstrap::run(None, None, vec![], false).unwrap();
    println!("NATIVE_STARTUP_RESTORED");
}
