use super::*;
use crate::{redact::Redactor, sessions::Store, test_support::Directory, tools::Tools};
use std::thread;
use std::time::Instant;

fn keep_shell_alive(tools: &Tools, marker: &str) {
    let result = tools.execute(
        "bash",
        &Value::object([
            ("command", Value::string(format!(": > {marker}; sleep 10"))),
            ("wait_ms", Value::number(0)),
        ])
        .encode(),
    );
    assert_eq!(
        result.get("status").and_then(Value::as_str),
        Some("running")
    );
    let started = Instant::now();
    while !tools.root().join(marker).exists() {
        assert!(started.elapsed() < Duration::from_secs(10), "{result:?}");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn deleting_a_session_does_not_break_another_shells_temporary_directory() {
    let project = Directory::new();
    let first_home = Directory::new();
    let temporary = Store::new(first_home.path().join("user data"), project.path())
        .unwrap()
        .temporary_area("123-42-0")
        .unwrap();
    let first_root = temporary.ensure().unwrap();
    let mut first = Tools::new(project.path()).unwrap();
    first.configure_output(first_home.path().join("logs"), Redactor::empty());
    first.configure_temporary(temporary);
    keep_shell_alive(&first, "first-ready");

    // A running command in another session retains its own temporary area.
    let other_home = Directory::new();
    let other_temporary = Store::new(other_home.path().join("user data"), project.path())
        .unwrap()
        .temporary_area("123-42-1")
        .unwrap();
    let other_root = other_temporary.ensure().unwrap();
    let mut other = Tools::new(project.path()).unwrap();
    other.configure_output(other_home.path().join("logs"), Redactor::empty());
    other.configure_temporary(other_temporary);
    keep_shell_alive(&other, "other-ready");
    drop(first);
    drop(first_home);
    assert!(!first_root.exists());

    let result = other.execute(
        "bash",
        &Value::object([(
            "command",
            Value::string("test -d \"$JECODE_TMP\" && printf tmp-present"),
        )])
        .encode(),
    );
    assert_eq!(result.get("stderr").and_then(Value::as_str), Some(""));
    assert_eq!(result.get("exit_code"), Some(&Value::number(0)));
    assert_eq!(
        result.get("stdout").and_then(Value::as_str),
        Some("tmp-present")
    );
    assert!(other_root.exists());
    // The other command keeps running across session deletion.
    assert!(
        other
            .observe_processes(Redactor::empty())
            .processes()
            .unwrap()
            .iter()
            .any(|process| process.active())
    );
}
