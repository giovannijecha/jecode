use std::process::{Command, Stdio};
#[path = "support/workspace.rs"]
mod fixture;

fn command(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_jecode"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn public_information_works_without_a_terminal() {
    let help = command(&["--help"]);
    assert!(help.status.success());
    assert!(help.stderr.is_empty());
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .contains("~/.jecode/v1/")
    );
    let version = command(&["--version"]);
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        format!("jecode {}", jecode::VERSION)
    );
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn local_account_commands_use_only_the_selected_fake_home() {
    let home = fixture::Fixture::new();
    home.write(".jecode/credentials.json", "untouched-legacy-fixture");
    if let Err(error) = jecode::state::Store::in_home(&home.0) {
        assert!(
            cfg!(target_os = "linux")
                && home.0.starts_with("/mnt/c/")
                && error.kind() == std::io::ErrorKind::Unsupported
        );
        return;
    }
    for option in ["--sessions", "--logout"] {
        let output = Command::new(env!("CARGO_BIN_EXE_jecode"))
            .arg(option)
            .env("USERPROFILE", &home.0)
            .env("HOME", &home.0)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
    }
    assert_eq!(
        std::fs::read_to_string(home.0.join(".jecode/credentials.json")).unwrap(),
        "untouched-legacy-fixture"
    );
    let saved = std::fs::read_to_string(home.0.join(".jecode/v1/credentials.json")).unwrap();
    assert!(saved.contains("signed_out"));
}

#[test]
fn invalid_input_is_rejected_without_echoing_arbitrary_text() {
    for args in [
        vec![],
        vec!["--api-key=secret-123\x1b[2J"],
        vec!["--help", "extra"],
        vec!["--demo"],
        vec!["--account"],
        vec!["--account", "--model", "gpt-5.6-luna"],
        vec!["--account", "--model", "gpt-5.6-terra"],
        vec!["--account", "--model", "secret-123"],
        vec!["--account", "--model"],
        vec!["--account", "--model", "gpt-5.6-luna", "extra"],
        vec!["--account", "--workspace"],
        vec!["--account", "--workspace", ""],
        vec!["--account", "--workspace", ".", "--workspace", "."],
        vec!["--account", "--workspace", ".", "--model", "gpt-5.6-luna"],
        vec!["--account", "--model", "gpt-5.6-luna", "--workspace", "."],
        vec![
            "--account",
            "--model",
            "gpt-5.6-luna",
            "--model",
            "gpt-5.6-terra",
        ],
    ] {
        let output = command(&args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(!error.contains("secret-123"));
        assert!(!error.contains('\x1b'));
    }
}
