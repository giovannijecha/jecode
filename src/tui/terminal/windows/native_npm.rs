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
    let result = native_script::pty(
        format!("{}\n{}", native_pty_source(), include_str!("native_npm.cs")),
        "RunNpm",
        &[
            &node.to_string_lossy(),
            &launcher.to_string_lossy(),
            &directory.path().to_string_lossy(),
        ],
        &directory,
        Duration::from_secs(60),
        true,
    );
    assert_eq!(
        result.exit_code,
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&result.stderr),
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(!result.timed_out);
}
