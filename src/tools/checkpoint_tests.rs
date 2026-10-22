//! Checkpoint protection as the model sees it through tool results.
use super::*;
use crate::test_support::Directory;
use std::thread;
use std::time::{Duration, Instant};

mod bash;
mod bash_results;
mod large;

struct Fixture {
    _home: Directory,
    root: Directory,
    tools: Tools,
}

fn fixture() -> Fixture {
    let home = Directory::new();
    let root = Directory::new();
    fs::write(root.path().join("notes.txt"), "first\n").unwrap();
    let mut tools = Tools::new(root.path()).unwrap();
    tools.configure_output(home.path().join("logs"), crate::redact::Redactor::empty());
    let session = Session::offline(home.path(), tools.root());
    session.begin("conversation-a", None);
    tools.configure_checkpoints(session);
    Fixture {
        _home: home,
        root,
        tools,
    }
}

fn call(tools: &Tools, name: &str, fields: &[(&str, Value)]) -> Value {
    let arguments = Value::Object(
        fields
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect(),
    );
    tools.execute(name, &arguments.encode())
}

fn text<'a>(value: &'a Value, path: &[&str]) -> &'a str {
    path.iter()
        .try_fold(value, |value, key| value.get(key))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{path:?} missing from {value:?}"))
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "condition timed out"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(windows)]
#[test]
fn inspect_uses_the_same_saved_name_as_restore_preview() {
    let fixture = fixture();
    fs::write(fixture.root.path().join("Case.txt"), b"original\n").unwrap();
    let captured = call(
        &fixture.tools,
        "checkpoint",
        &[
            ("action", Value::string("capture")),
            ("paths", Value::Array(vec![Value::string("Case.txt")])),
        ],
    );
    let group = text(&captured, &["group"]).to_string();
    fs::write(fixture.root.path().join("Case.txt"), b"changed\n").unwrap();

    let inspected = call(
        &fixture.tools,
        "checkpoint",
        &[
            ("action", Value::string("inspect")),
            ("ref", Value::string(&group)),
            ("paths", Value::Array(vec![Value::string("case.txt")])),
        ],
    );
    let states = inspected.get("paths").and_then(Value::as_array).unwrap();
    let state = &states[0];
    assert_eq!(text(state, &["path"]), "Case.txt", "{inspected:?}");
    assert!(
        text(state, &["at_ref"]).starts_with("file"),
        "{inspected:?}"
    );
    assert!(
        text(state, &["current"]).starts_with("file"),
        "{inspected:?}"
    );
    assert_eq!(
        text(state, &["read"]),
        format!("checkpoint:{group}:Case.txt"),
        "{inspected:?}"
    );
    let planned = inspected
        .get("restore_would")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(text(&planned[0], &["path"]), "Case.txt", "{inspected:?}");
}

#[test]
fn model_milestone_cannot_pin_history() {
    let fixture = fixture();
    let tools = &fixture.tools;
    let status = call(tools, "checkpoint", &[("action", Value::string("status"))]);
    let pinned = status.get("pinned").and_then(Value::as_usize);
    assert_eq!(pinned, Some(0), "{status:?}");
    let rejected = call(
        tools,
        "checkpoint",
        &[
            ("action", Value::string("milestone")),
            ("label", Value::string("model pin")),
            ("pin", Value::Bool(true)),
        ],
    );
    assert!(rejected.get("error").is_some(), "{rejected:?}");
    let after = call(tools, "checkpoint", &[("action", Value::string("status"))]);
    assert_eq!(after.get("pinned").and_then(Value::as_usize), pinned);

    let definitions = crate::tools::schema::definitions();
    let checkpoint = definitions
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| {
            tool.get("function").and_then(|f| f.get("name")) == Some(&Value::string("checkpoint"))
        })
        .unwrap();
    assert!(
        checkpoint
            .get("function")
            .unwrap()
            .get("parameters")
            .unwrap()
            .get("properties")
            .unwrap()
            .get("pin")
            .is_none()
    );
}

#[test]
fn file_tools_return_saved_references_that_read_and_restore_use() {
    let fixture = fixture();
    let tools = &fixture.tools;
    let written = call(
        tools,
        "write",
        &[
            ("path", Value::string("notes.txt")),
            ("content", Value::string("second\n")),
        ],
    );
    assert_eq!(text(&written, &["checkpoint", "status"]), "saved");
    let before = text(&written, &["checkpoint", "before"]).to_string();
    assert!(before.starts_with("cp-"), "{written:?}");
    let edited = call(
        tools,
        "edit",
        &[
            ("path", Value::string("notes.txt")),
            ("old_text", Value::string("second")),
            ("new_text", Value::string("third")),
        ],
    );
    assert_eq!(
        text(&edited, &["checkpoint", "status"]),
        "saved",
        "{edited:?}"
    );

    // History is readable through read and refused by the writing tools.
    let old = call(
        tools,
        "read",
        &[(
            "path",
            Value::string(format!("checkpoint:{before}:notes.txt")),
        )],
    );
    assert!(old.encode().contains("first"), "{old:?}");
    let refused = call(
        tools,
        "write",
        &[
            (
                "path",
                Value::string(format!("checkpoint:{before}:notes.txt")),
            ),
            ("content", Value::string("x")),
        ],
    );
    assert!(text(&refused, &["error"]).contains("read-only"));

    // Generated trees are outside the default scope and say so.
    fs::create_dir_all(fixture.root.path().join("node_modules/pkg")).unwrap();
    let generated = call(
        tools,
        "write",
        &[
            ("path", Value::string("node_modules/pkg/index.js")),
            ("content", Value::string("x\n")),
        ],
    );
    assert_eq!(
        text(&generated, &["checkpoint", "status"]),
        "not_tracked",
        "{generated:?}"
    );

    let listed = call(
        tools,
        "checkpoint",
        &[
            ("action", Value::string("list")),
            ("paths", Value::Array(vec![Value::string("notes.txt")])),
        ],
    );
    // The first write records the original, followed by write and edit.
    let history = &listed.get("paths").and_then(Value::as_array).unwrap()[0];
    assert_eq!(
        history.get("total").and_then(Value::as_usize),
        Some(3),
        "{listed:?}"
    );
    let restored = call(
        tools,
        "checkpoint",
        &[
            ("action", Value::string("restore")),
            ("ref", Value::string(&before)),
            ("paths", Value::Array(vec![Value::string("notes.txt")])),
        ],
    );
    assert_eq!(
        fs::read_to_string(fixture.root.path().join("notes.txt")).unwrap(),
        "first\n"
    );
    let undo = restored
        .get("undo")
        .unwrap_or_else(|| panic!("{restored:?}"));
    let undo_ref = text(undo, &["ref"]).to_string();
    call(
        tools,
        "checkpoint",
        &[
            ("action", Value::string("restore")),
            ("ref", Value::string(&undo_ref)),
            ("paths", Value::Array(vec![Value::string("notes.txt")])),
        ],
    );
    assert_eq!(
        fs::read_to_string(fixture.root.path().join("notes.txt")).unwrap(),
        "third\n"
    );
    assert!(!crate::events::tool_summary("write", &written).contains("unprotected"));
}

#[test]
fn successful_temporary_writes_and_edits_report_their_checkpoint_exclusion() {
    let mut fixture = fixture();
    let store =
        crate::sessions::Store::new(fixture._home.path().to_path_buf(), fixture.root.path())
            .unwrap();
    fixture
        .tools
        .configure_temporary(store.temporary_area("123-1-0").unwrap());
    let written = call(
        &fixture.tools,
        "write",
        &[
            ("path", Value::string("tmp:notes.txt")),
            ("content", Value::string("first")),
        ],
    );
    assert_eq!(text(&written, &["checkpoint", "status"]), "not_tracked");
    assert!(text(&written, &["checkpoint", "reason"]).contains("temporary"));
    let edited = call(
        &fixture.tools,
        "edit",
        &[
            ("path", Value::string("tmp:notes.txt")),
            ("old_text", Value::string("first")),
            ("new_text", Value::string("second")),
        ],
    );
    assert_eq!(text(&edited, &["checkpoint", "status"]), "not_tracked");
    let unchanged = call(
        &fixture.tools,
        "edit",
        &[
            ("path", Value::string("tmp:notes.txt")),
            ("old_text", Value::string("missing")),
            ("new_text", Value::string("ignored")),
        ],
    );
    assert!(unchanged.get("checkpoint").is_none());
}

#[test]
fn stopping_a_conversation_clears_its_background_checkpoint_count() {
    let fixture = fixture();
    let session = fixture.tools.checkpoints().unwrap();
    session.set_background(1);
    let status = call(
        &fixture.tools,
        "checkpoint",
        &[("action", Value::string("status"))],
    );
    assert_eq!(
        status.get("background_processes").and_then(Value::as_usize),
        Some(1)
    );
    fixture.tools.stop_processes();
    let status = call(
        &fixture.tools,
        "checkpoint",
        &[("action", Value::string("status"))],
    );
    assert_eq!(
        status.get("background_processes").and_then(Value::as_usize),
        Some(0)
    );
}

#[test]
fn a_cancelled_wait_keeps_protection_for_the_running_command() {
    let fixture = fixture();
    let tools = &fixture.tools;
    let cancellation = Cancellation::default();
    let result = thread::scope(|scope| {
        scope.spawn(|| {
            wait_for(|| fixture.root.path().join("started").exists());
            cancellation.cancel();
        });
        tools.execute_with_cancel(
            "bash",
            &Value::object([
                (
                    "command",
                    Value::string("touch started; until [ -e go ]; do sleep 0.02; done"),
                ),
                ("protect", Value::Array(vec![Value::string("started")])),
                ("wait_ms", Value::number(30_000)),
            ])
            .encode(),
            &cancellation,
        )
    });
    assert_eq!(
        result.get("wait_cancelled"),
        Some(&Value::Bool(true)),
        "{result:?}"
    );
    assert_eq!(text(&result, &["checkpoint", "status"]), "partial");
    let checkpoint = result.get("checkpoint").unwrap();
    let before = text(checkpoint, &["before"]).to_string();
    assert!(text(checkpoint, &["group"]).starts_with("cp-"));
    assert!(checkpoint.get("after").is_none(), "{checkpoint:?}");
    assert!(
        text(checkpoint, &["error"])
            .to_lowercase()
            .contains("cancel"),
        "{checkpoint:?}"
    );
    assert!(text(&result, &["checkpoint", "note"]).contains("still running"));
    fs::write(fixture.root.path().join("go"), "").unwrap();
    tools.stop_processes();
    let restored = call(
        tools,
        "checkpoint",
        &[
            ("action", Value::string("restore")),
            ("ref", Value::string(before)),
            ("paths", Value::Array(vec![Value::string("started")])),
            ("expected", Value::string("latest")),
        ],
    );
    assert!(restored.get("error").is_none(), "{restored:?}");
    assert!(!fixture.root.path().join("started").exists());
}

#[test]
fn an_unavailable_archive_never_blocks_a_tool_call() {
    let home = Directory::new();
    let root = Directory::new();
    // The configuration directory cannot hold an archive.
    let blocked = home.path().join("file");
    fs::write(&blocked, "").unwrap();
    let mut tools = Tools::new(root.path()).unwrap();
    tools.configure_output(home.path().join("logs"), crate::redact::Redactor::empty());
    tools.configure_checkpoints(Session::offline(&blocked, root.path()));

    let written = call(
        &tools,
        "write",
        &[
            ("path", Value::string("a.txt")),
            ("content", Value::string("a\n")),
        ],
    );
    assert_eq!(
        fs::read_to_string(root.path().join("a.txt")).unwrap(),
        "a\n"
    );
    assert_eq!(text(&written, &["checkpoint", "status"]), "unprotected");
    assert!(
        text(&written, &["checkpoint", "reason"]).starts_with("the previous state was not saved")
    );
    let summary = crate::events::tool_summary("write", &written);
    assert!(
        summary.contains("unprotected: the previous state was not saved"),
        "{summary}"
    );

    let arguments = Value::object([
        ("command", Value::string("printf b > b.txt")),
        ("protect", Value::Array(vec![Value::string("b.txt")])),
    ])
    .encode();
    let begun = tools.begin("bash", &arguments);
    assert!(begun.note().unwrap().starts_with("checkpoint none · "));
    let result = tools
        .execute_begun(begun, "bash", &arguments, &Cancellation::default())
        .result;
    assert_eq!(result.get("exit_code").and_then(Value::as_usize), Some(0));
    assert_eq!(text(&result, &["checkpoint", "status"]), "none");
    assert_eq!(fs::read_to_string(root.path().join("b.txt")).unwrap(), "b");
    let status = call(&tools, "checkpoint", &[("action", Value::string("status"))]);
    assert!(
        text(&status, &["error"]).contains("checkpoints are unavailable"),
        "{status:?}"
    );
}

#[test]
fn checkpoint_calls_without_a_session_explain_themselves() {
    let root = Directory::new();
    let tools = Tools::new(root.path()).unwrap();
    let result = call(&tools, "checkpoint", &[("action", Value::string("status"))]);
    assert!(text(&result, &["error"]).contains("not enabled"));
    let written = call(
        &tools,
        "write",
        &[
            ("path", Value::string("a.txt")),
            ("content", Value::string("a")),
        ],
    );
    assert!(written.get("checkpoint").is_none());
}
