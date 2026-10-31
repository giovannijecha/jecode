use super::*;

#[test]
#[ignore = "requires Windows ConPTY and a staged npm package"]
fn npm_launcher_native_tui() {
    let node = std::path::PathBuf::from(
        std::env::var_os("JECODE_NPM_NODE").expect("set JECODE_NPM_NODE to the absolute node.exe"),
    );
    let launcher = std::path::PathBuf::from(
        std::env::var_os("JECODE_NPM_LAUNCHER")
            .expect("set JECODE_NPM_LAUNCHER to the packed bin/jecode.cjs"),
    );
    assert!(node.is_absolute() && node.is_file());
    assert!(launcher.is_absolute() && launcher.is_file());
    let directory = Directory::new();
    let script = format!(
        "Add-Type -TypeDefinition @'\n{}\n{}\n'@\n[JecodeTestPty]::RunNpm('{}', '{}', '{}')\n",
        native_pty_source(),
        include_str!("native_npm.cs"),
        node.to_string_lossy().replace('\'', "''"),
        launcher.to_string_lossy().replace('\'', "''"),
        directory.path().to_string_lossy().replace('\'', "''"),
    );
    let script_path = directory.path().join("npm-native-pty.ps1");
    std::fs::write(&script_path, script).unwrap();
    let result = process::run_observed(
        Command::new("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
            .arg(&script_path)
            .env("TEMP", directory.path())
            .env("TMP", directory.path())
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("OPENROUTER_MODEL"),
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
