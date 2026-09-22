#![cfg(any(windows, target_os = "linux"))]
#[path = "support/workspace.rs"]
mod fixture;
#[path = "support/sessions.rs"]
mod sessions;
use std::process::{Command, Stdio};

#[test]
fn session_cards_are_ordered_readable_and_browsing_is_non_mutating() {
    let home = fixture::Fixture::new();
    home.write(".jecode/credentials.json", "untouched legacy fixture");
    if let Err(error) = sessions::populate(&home.0) {
        assert!(
            cfg!(target_os = "linux")
                && home.0.starts_with("/mnt/c/")
                && error.kind() == std::io::ErrorKind::Unsupported
        );
        return;
    }
    let root = home.0.join(".jecode/v1/sessions");
    let files = [sessions::OLDER, sessions::NEWER].map(|id| root.join(format!("{id}.json")));
    let before = files.each_ref().map(|file| std::fs::read(file).unwrap());
    let output = Command::new(env!("CARGO_BIN_EXE_jecode"))
        .arg("--sessions")
        .env("USERPROFILE", &home.0)
        .env("HOME", &home.0)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    assert!(output.stderr.is_empty());
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.contains("1. Read settings.rs"), "{output}");
    assert!(output.contains("2. Older conversation"), "{output}");
    assert!(output.contains("1 turn · gpt-5.6-luna"), "{output}");
    assert!(output.contains("unavailable-workspace"), "{output}");
    assert!(
        output.contains(&format!("ID: {}", sessions::NEWER)),
        "{output}"
    );
    assert!(!output.contains('\x1b'));
    for (file, expected) in files.iter().zip(before) {
        assert_eq!(std::fs::read(file).unwrap(), expected);
    }
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
    assert!(!home.0.join(".jecode/v1/credentials.json").exists());
    assert_eq!(
        std::fs::read_to_string(home.0.join(".jecode/credentials.json")).unwrap(),
        "untouched legacy fixture"
    );
    let no_terminal = Command::new(env!("CARGO_BIN_EXE_jecode"))
        .arg("--resume")
        .env("USERPROFILE", &home.0)
        .env("HOME", &home.0)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(no_terminal.status.code(), Some(2));
    assert!(no_terminal.stdout.is_empty());
    assert!(
        String::from_utf8(no_terminal.stderr)
            .unwrap()
            .contains("interactive terminal")
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
}
