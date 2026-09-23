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
        .arg("--workspace")
        .arg(&home.0)
        .env("USERPROFILE", &home.0)
        .env("HOME", &home.0)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    assert!(output.stderr.is_empty());
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.contains("1. Read settings.rs"), "{output}");
    assert!(!output.contains("Older conversation"), "{output}");
    assert!(output.contains("1 turn · gpt-5.6-luna"), "{output}");
    assert!(
        output.contains(&home.0.to_string_lossy().to_string()),
        "{output}"
    );
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
        .arg("--workspace")
        .arg(&home.0)
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

    let other = Command::new(env!("CARGO_BIN_EXE_jecode"))
        .arg("sessions")
        .arg("--workspace")
        .arg(home.0.join(sessions::OTHER))
        .env("USERPROFILE", &home.0)
        .env("HOME", &home.0)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(other.status.success());
    let other = String::from_utf8(other.stdout).unwrap();
    assert!(other.contains("Older conversation"), "{other}");
    assert!(!other.contains("Read settings.rs"), "{other}");

    let mismatch = Command::new(env!("CARGO_BIN_EXE_jecode"))
        .arg("resume")
        .arg(sessions::OLDER)
        .arg("--workspace")
        .arg(&home.0)
        .env("USERPROFILE", &home.0)
        .env("HOME", &home.0)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(mismatch.status.code(), Some(2));
    let error = String::from_utf8(mismatch.stderr).unwrap();
    assert!(
        error.contains("another directory") && error.contains("--workspace"),
        "{error}"
    );

    let equivalent = Command::new(env!("CARGO_BIN_EXE_jecode"))
        .arg("sessions")
        .arg("--workspace")
        .arg(home.0.join(sessions::OTHER).join(".."))
        .env("USERPROFILE", &home.0)
        .env("HOME", &home.0)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(equivalent.status.success());
    assert!(
        String::from_utf8(equivalent.stdout)
            .unwrap()
            .contains("Read settings.rs")
    );

    let unavailable = Command::new(env!("CARGO_BIN_EXE_jecode"))
        .arg("sessions")
        .arg("--workspace")
        .arg(home.0.join("missing-directory"))
        .env("USERPROFILE", &home.0)
        .env("HOME", &home.0)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(unavailable.status.code(), Some(2));
    assert!(
        String::from_utf8(unavailable.stderr)
            .unwrap()
            .contains("selected working directory")
    );

    let legacy_id = "s-0000000000000003-00000000-00000000";
    let mut legacy = jecode::json::parse(
        &std::fs::read_to_string(&files[0]).unwrap(),
        Default::default(),
    )
    .unwrap();
    let jecode::json::Value::Object(fields) = &mut legacy else {
        unreachable!()
    };
    fields.insert("id".into(), jecode::json::Value::String(legacy_id.into()));
    fields.insert("workspace".into(), jecode::json::Value::Null);
    fields.remove("directory");
    fields.insert(
        "file_access".into(),
        jecode::json::Value::String("workspace".into()),
    );
    let legacy_path = root.join(format!("{legacy_id}.json"));
    jecode::state::Store::in_home(&home.0)
        .unwrap()
        .directory("sessions")
        .unwrap()
        .replace(
            &format!("{legacy_id}.json"),
            &jecode::json::encode(&legacy, 65536).unwrap(),
        )
        .unwrap();
    let legacy_before = std::fs::read(&legacy_path).unwrap();
    let legacy_resume = Command::new(env!("CARGO_BIN_EXE_jecode"))
        .arg("resume")
        .arg(legacy_id)
        .arg("--workspace")
        .arg(&home.0)
        .env("USERPROFILE", &home.0)
        .env("HOME", &home.0)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(legacy_resume.status.code(), Some(2));
    let diagnostic = String::from_utf8(legacy_resume.stderr).unwrap();
    assert!(
        diagnostic.contains("origin cannot be inferred"),
        "{diagnostic}"
    );
    assert_eq!(std::fs::read(legacy_path).unwrap(), legacy_before);
}
