use super::*;
use crate::{test_support::Directory, tools::Tools};
use std::fs;

fn call(tools: &Tools, name: &str, arguments: Value) -> Value {
    tools.execute(name, &arguments.encode())
}

fn read(tools: &Tools, path: &str) -> Value {
    call(
        tools,
        "read",
        Value::object([("path", Value::string(path))]),
    )
}

fn bash(tools: &Tools, command: &str) -> Value {
    call(
        tools,
        "bash",
        Value::object([
            ("command", Value::string(command)),
            ("check", Value::Bool(true)),
        ]),
    )
}

fn changes(result: &Value) -> &[Value] {
    result.get("file_changes").unwrap().as_array().unwrap()
}

#[test]
fn a_successful_formatter_reports_content_changes_and_the_original_read_reference() {
    let directory = Directory::new();
    fs::write(directory.path().join("protected.rs"), "assert_eq!(a,b);").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    let original = read(&tools, "protected.rs");
    tools.record_file_history(3, &original);
    let result = bash(&tools, "printf 'assert_eq!(a, b);' > protected.rs");
    assert_eq!(
        result.get("check_status").and_then(Value::as_str),
        Some("passed")
    );
    assert_eq!(changes(&result).len(), 1);
    let changed = &changes(&result)[0];
    assert_eq!(
        changed.get("path").and_then(Value::as_str),
        Some("protected.rs")
    );
    assert_eq!(
        changed.get("source").and_then(Value::as_str),
        Some("during_command")
    );
    assert_eq!(
        changed
            .get("first_observed")
            .unwrap()
            .get("history_reference")
            .and_then(Value::as_str),
        Some("history:3")
    );
    assert_ne!(
        changed.get("before").unwrap().get("fingerprint"),
        changed.get("after").unwrap().get("fingerprint")
    );
    tools.record_file_history(5, &result);
    assert!(changes(&bash(&tools, "true")).is_empty());
    let restored = bash(&tools, "printf 'assert_eq!(a,b);' > protected.rs");
    assert_eq!(
        changes(&restored)[0].get("restored_to_first_observed"),
        Some(&Value::Bool(true))
    );
}

#[test]
fn content_changes_are_seen_even_when_size_and_timestamp_are_preserved() {
    let directory = Directory::new();
    let path = directory.path().join("source");
    fs::write(&path, "old").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    read(&tools, "source");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, "new").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let result = bash(&tools, "true");
    assert_eq!(changes(&result).len(), 1);
    assert_eq!(
        changes(&result)[0].get("source").and_then(Value::as_str),
        Some("before_command")
    );
}

#[test]
fn failed_commands_report_deletion_and_resume_retains_the_first_observed_version() {
    let directory = Directory::new();
    fs::write(directory.path().join("protected"), "keep").unwrap();
    let first = Tools::new(directory.path()).unwrap();
    let original = read(&first, "protected");
    first.record_file_history(1, &original);
    let deleted = bash(&first, "rm protected; exit 7");
    assert_eq!(deleted.get("exit_code"), Some(&Value::number(7)));
    assert_eq!(
        changes(&deleted)[0]
            .get("after")
            .unwrap()
            .get("state")
            .and_then(Value::as_str),
        Some("missing")
    );
    let messages = [original, deleted]
        .iter()
        .map(|result| {
            Value::object([
                ("role", Value::string("tool")),
                ("content", Value::string(result.encode())),
            ])
        })
        .collect::<Vec<_>>();
    let resumed = Tools::new(directory.path()).unwrap();
    resumed.restore_watches(&messages).unwrap();
    let result = bash(&resumed, "printf keep > protected");
    assert_eq!(
        changes(&result)[0].get("restored_to_first_observed"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        changes(&result)[0]
            .get("first_observed")
            .unwrap()
            .get("history_reference")
            .and_then(Value::as_str),
        Some("history:0")
    );
}

#[test]
fn explicit_watch_covers_bash_inspections_and_invalid_scope_prevents_execution() {
    let directory = Directory::new();
    fs::write(directory.path().join("protected"), "keep").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    let result = call(
        &tools,
        "bash",
        Value::object([
            ("command", Value::string("printf changed > protected")),
            ("watch", Value::Array(vec![Value::string("protected")])),
        ]),
    );
    assert_eq!(changes(&result).len(), 1);
    assert_eq!(
        result.get("file_tracking").unwrap().get("watched_files"),
        Some(&Value::number(1))
    );
    for watch in [
        Value::string("protected"),
        Value::Array(vec![Value::number(1)]),
        Value::Array(vec![Value::string("../outside")]),
        Value::Array(vec![Value::string(".")]),
        Value::Array(vec![Value::string("tmp:fixture")]),
    ] {
        let result = call(
            &tools,
            "bash",
            Value::object([
                ("command", Value::string("printf unexpected > marker")),
                ("watch", watch),
            ]),
        );
        assert!(result.get("error").is_some());
        assert!(!directory.path().join("marker").exists());
    }
}

#[test]
fn explicit_file_edits_advance_the_version_and_do_not_look_like_indirect_changes() {
    let directory = Directory::new();
    fs::write(directory.path().join("source"), "old").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    read(&tools, "source");
    let edited = call(
        &tools,
        "edit",
        Value::object([
            ("path", Value::string("source")),
            ("old_text", Value::string("old")),
            ("new_text", Value::string("new")),
        ]),
    );
    assert_eq!(
        changes(&edited)[0].get("source").and_then(Value::as_str),
        Some("file_tool")
    );
    assert!(changes(&bash(&tools, "true")).is_empty());
}

#[test]
fn cancellation_and_replaced_directories_report_incomplete_comparison() {
    let directory = Directory::new();
    let path = directory.path().join("source");
    fs::write(&path, "old").unwrap();
    let tools = Tools::new(directory.path()).unwrap();
    read(&tools, "source");
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let cancelled =
        tools.execute_with_cancel("bash", r#"{"command":"true","check":true}"#, &cancellation);
    assert_eq!(
        cancelled
            .get("file_tracking")
            .unwrap()
            .get("status")
            .and_then(Value::as_str),
        Some("incomplete")
    );
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    let result = bash(&tools, "true");
    assert_eq!(
        result
            .get("file_tracking")
            .unwrap()
            .get("status")
            .and_then(Value::as_str),
        Some("incomplete")
    );
    assert!(changes(&result).is_empty());
}

#[test]
fn absolute_temporary_writes_and_saved_output_reads_do_not_invalidate_project_checks() {
    let directory = Directory::new();
    let home = Directory::new();
    let temporary = crate::sessions::Store::new(home.path().to_path_buf(), directory.path())
        .unwrap()
        .temporary_area("123-1-0")
        .unwrap();
    let temporary_root = temporary.ensure().unwrap();
    let mut tools = Tools::new(directory.path()).unwrap();
    tools.configure_output(home.path().join("output"), crate::redact::Redactor::empty());
    tools.configure_temporary(temporary);
    let checked = bash(&tools, "printf checked");
    let saved = checked.get("stdout_file").and_then(Value::as_str).unwrap();
    assert!(read(&tools, saved).get("file_observations").is_none());
    let arguments = Value::object([
        (
            "path",
            Value::string(temporary_root.join("probe").to_string_lossy()),
        ),
        ("content", Value::string("disposable")),
    ]);
    let written = call(&tools, "write", arguments.clone());
    assert_eq!(
        written.get("file_scope").and_then(Value::as_str),
        Some("temporary")
    );
    assert!(written.get("file_observations").is_none());
    let mut evidence = crate::context::evidence::Evidence::default();
    evidence.record(
        3,
        "bash",
        &Value::object([("command", Value::string("check"))]),
        &checked,
    );
    evidence.record(5, "write", &arguments, &written);
    assert!(!evidence.needs_attention());
    assert_eq!(
        evidence.value().get("last_tracked_change"),
        Some(&Value::Null)
    );
    assert_eq!(
        bash(&tools, "true")
            .get("file_tracking")
            .unwrap()
            .get("watched_files"),
        Some(&Value::number(0))
    );
}

#[cfg(unix)]
#[test]
fn a_replaced_symlink_is_not_followed_outside_the_working_directory() {
    let directory = Directory::new();
    let workspace = directory.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let path = workspace.join("source");
    fs::write(&path, "inside").unwrap();
    fs::write(directory.path().join("secret"), "outside-secret").unwrap();
    let tools = Tools::new(&workspace).unwrap();
    read(&tools, "source");
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(directory.path().join("secret"), &path).unwrap();
    let result = bash(&tools, "true");
    assert_eq!(
        result
            .get("file_tracking")
            .unwrap()
            .get("status")
            .and_then(Value::as_str),
        Some("incomplete")
    );
    assert!(!result.encode().contains("outside-secret"));
}
