use super::*;
use crate::{redact::Redactor, sessions::Store, test_support::Directory, tools::Tools};
use std::thread;
use std::time::Instant;

fn keep_shell_alive(tools: &Tools, marker: &str) {
    let result = tools.execute(
        "bash",
        &Value::object([
            (
                "command",
                Value::string(format!(": > {marker}; while :; do sleep 1; done")),
            ),
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
fn deleting_a_session_does_not_break_another_shells_temporary_mount() {
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

    // Keep the MSYS runtime alive while its first session and TEMP disappear.
    let mut other = Tools::new(project.path()).unwrap();
    other.configure_output(project.path().join("logs"), Redactor::empty());
    keep_shell_alive(&other, "other-ready");
    drop(first);
    drop(first_home);
    assert!(!first_root.exists());

    let result = other.execute(
        "bash",
        &Value::object([(
            "command",
            Value::string("test -d /tmp && printf tmp-present"),
        )])
        .encode(),
    );
    assert_eq!(result.get("stderr").and_then(Value::as_str), Some(""));
    assert_eq!(result.get("exit_code"), Some(&Value::number(0)));
    assert_eq!(
        result.get("stdout").and_then(Value::as_str),
        Some("tmp-present")
    );
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
