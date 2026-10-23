//! Per-call recovery as seen by the model tools.
use super::*;
use crate::test_support::Directory;
use std::io::Read;
use std::time::Duration;

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
    session.begin("conversation-a");
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

fn saved_bytes(tools: &Tools, reference: &str, path: &str) -> Vec<u8> {
    let store = tools.checkpoints().unwrap().store().unwrap();
    let loaded = store.load(reference, &Cancellation::default()).unwrap();
    let mut bytes = Vec::new();
    loaded
        .reader(loaded.snapshot.entries.get(path).unwrap())
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}

#[test]
fn write_and_edit_each_save_their_own_exact_preimage() {
    let fixture = fixture();
    let written = call(
        &fixture.tools,
        "write",
        &[
            ("path", Value::string("notes.txt")),
            ("content", Value::string("second\n")),
        ],
    );
    assert_eq!(text(&written, &["checkpoint", "status"]), "saved");
    let first = text(&written, &["checkpoint", "ref"]);
    assert!(first.starts_with("rc-"));
    assert_eq!(saved_bytes(&fixture.tools, first, "notes.txt"), b"first\n");
    assert!(
        written
            .get("checkpoint")
            .unwrap()
            .get("expires_at")
            .is_some()
    );

    let edited = call(
        &fixture.tools,
        "edit",
        &[
            ("path", Value::string("notes.txt")),
            ("old_text", Value::string("second")),
            ("new_text", Value::string("third")),
        ],
    );
    assert_eq!(text(&edited, &["checkpoint", "status"]), "saved");
    let second = text(&edited, &["checkpoint", "ref"]);
    assert_ne!(first, second);
    assert_eq!(
        saved_bytes(&fixture.tools, second, "notes.txt"),
        b"second\n"
    );
    assert_eq!(
        fs::read(fixture.root.path().join("notes.txt")).unwrap(),
        b"third\n"
    );
}

#[test]
fn bash_receipt_is_the_prelaunch_capture_even_when_command_fails() {
    let fixture = fixture();
    let result = call(
        &fixture.tools,
        "bash",
        &[
            (
                "command",
                Value::string("printf changed > notes.txt; exit 7"),
            ),
            ("protect", Value::Array(vec![Value::string("notes.txt")])),
        ],
    );
    assert_eq!(result.get("exit_code").and_then(Value::as_usize), Some(7));
    assert_eq!(text(&result, &["checkpoint", "status"]), "saved");
    let reference = text(&result, &["checkpoint", "ref"]);
    assert_eq!(
        saved_bytes(&fixture.tools, reference, "notes.txt"),
        b"first\n"
    );
    assert_eq!(
        fs::read(fixture.root.path().join("notes.txt")).unwrap(),
        b"changed"
    );
    for removed in ["before", "after", "group", "changed"] {
        assert!(result.get("checkpoint").unwrap().get(removed).is_none());
    }
}

#[test]
fn cancelled_background_wait_keeps_the_prelaunch_receipt() {
    let fixture = fixture();
    let cancellation = Cancellation::default();
    let interrupt = cancellation.clone();
    let observer = fixture
        .tools
        .observe_processes(crate::redact::Redactor::empty());
    let worker = std::thread::spawn(move || {
        let started = std::time::Instant::now();
        while observer
            .processes()
            .is_none_or(|processes| processes.is_empty())
        {
            if started.elapsed() >= Duration::from_secs(10) {
                interrupt.cancel();
                panic!("Process was not registered before the cancellation deadline");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        interrupt.cancel();
    });
    let result = fixture.tools.execute_with_cancel(
        "bash",
        &Value::object([
            (
                "command",
                Value::string("sleep 30; printf changed > notes.txt"),
            ),
            ("wait_ms", Value::number(15_000)),
            ("protect", Value::Array(vec![Value::string("notes.txt")])),
        ])
        .encode(),
        &cancellation,
    );
    worker.join().unwrap();
    fixture.tools.stop_processes();
    assert_eq!(
        result.get("wait_cancelled"),
        Some(&Value::Bool(true)),
        "{result:?}"
    );
    assert_eq!(text(&result, &["checkpoint", "status"]), "saved");
    let reference = text(&result, &["checkpoint", "ref"]);
    assert_eq!(
        saved_bytes(&fixture.tools, reference, "notes.txt"),
        b"first\n"
    );
}

#[test]
fn missed_edit_does_not_create_a_snapshot() {
    let fixture = fixture();
    let result = call(
        &fixture.tools,
        "edit",
        &[
            ("path", Value::string("notes.txt")),
            ("old_text", Value::string("missing")),
            ("new_text", Value::string("other")),
        ],
    );
    assert!(result.get("checkpoint").is_none());
    assert!(
        fixture
            .tools
            .checkpoints()
            .unwrap()
            .store()
            .unwrap()
            .list(10, &Cancellation::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn failed_installation_still_returns_its_saved_preimage() {
    let fixture = fixture();
    let path = fixture.root.path().join("notes.txt");
    let original_permissions = fs::metadata(&path).unwrap().permissions();
    let mut permissions = original_permissions.clone();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();
    let result = call(
        &fixture.tools,
        "write",
        &[
            ("path", Value::string("notes.txt")),
            ("content", Value::string("replacement")),
        ],
    );
    fs::set_permissions(&path, original_permissions).unwrap();
    assert!(text(&result, &["error"]).contains("read-only"));
    assert_eq!(text(&result, &["checkpoint", "status"]), "saved");
    let reference = text(&result, &["checkpoint", "ref"]);
    assert_eq!(
        saved_bytes(&fixture.tools, reference, "notes.txt"),
        b"first\n"
    );
    assert_eq!(fs::read(path).unwrap(), b"first\n");
}

#[test]
fn checkpoint_tool_inspects_restores_and_returns_a_usable_undo() {
    let fixture = fixture();
    let written = call(
        &fixture.tools,
        "write",
        &[
            ("path", Value::string("notes.txt")),
            ("content", Value::string("second\n")),
        ],
    );
    let reference = text(&written, &["checkpoint", "ref"]);
    let inspected = call(
        &fixture.tools,
        "checkpoint",
        &[
            ("action", Value::string("inspect")),
            ("ref", Value::string(reference)),
        ],
    );
    let expected = text(&inspected, &["expected"]);
    let unsafe_restore = call(
        &fixture.tools,
        "checkpoint",
        &[
            ("action", Value::string("restore")),
            ("ref", Value::string(reference)),
        ],
    );
    assert!(text(&unsafe_restore, &["error"]).contains("expected"));
    assert_eq!(
        fs::read(fixture.root.path().join("notes.txt")).unwrap(),
        b"second\n"
    );
    let restored = call(
        &fixture.tools,
        "checkpoint",
        &[
            ("action", Value::string("restore")),
            ("ref", Value::string(reference)),
            ("expected", Value::string(expected)),
        ],
    );
    assert_eq!(
        restored.get("restored_count").and_then(Value::as_usize),
        Some(1)
    );
    assert_eq!(
        fs::read(fixture.root.path().join("notes.txt")).unwrap(),
        b"first\n"
    );
    let undo = restored.get("undo").unwrap();
    let redone = call(
        &fixture.tools,
        "checkpoint",
        &[
            ("action", Value::string("restore")),
            ("ref", Value::string(text(undo, &["ref"]))),
            ("expected", Value::string(text(undo, &["expected"]))),
            ("paths", undo.get("paths").unwrap().clone()),
        ],
    );
    assert_eq!(
        redone.get("restored_count").and_then(Value::as_usize),
        Some(1)
    );
    assert_eq!(
        fs::read(fixture.root.path().join("notes.txt")).unwrap(),
        b"second\n"
    );
}

#[test]
fn restoring_nested_agents_instructions_requests_reload_but_legacy_name_does_not() {
    let fixture = fixture();
    fs::create_dir(fixture.root.path().join("src")).unwrap();
    fs::write(fixture.root.path().join("src/AGENTS.md"), "Original rule\n").unwrap();
    let written = call(
        &fixture.tools,
        "write",
        &[
            ("path", Value::string("src/AGENTS.md")),
            ("content", Value::string("Changed rule\n")),
        ],
    );
    let reference = text(&written, &["checkpoint", "ref"]);
    let restored = call(
        &fixture.tools,
        "checkpoint",
        &[
            ("action", Value::string("restore")),
            ("ref", Value::string(reference)),
            ("expected", Value::string("latest")),
        ],
    );
    assert_eq!(
        restored.get("restored_count").and_then(Value::as_usize),
        Some(1)
    );
    assert!(
        fixture
            .tools
            .checkpoints()
            .unwrap()
            .take_restored_instructions()
    );
    assert!(
        !fixture
            .tools
            .checkpoints()
            .unwrap()
            .take_restored_instructions()
    );

    fs::write(fixture.root.path().join("JECODE.md"), "Legacy rule\n").unwrap();
    let written = call(
        &fixture.tools,
        "write",
        &[
            ("path", Value::string("JECODE.md")),
            ("content", Value::string("Changed legacy rule\n")),
        ],
    );
    let restored = call(
        &fixture.tools,
        "checkpoint",
        &[
            ("action", Value::string("restore")),
            ("ref", Value::string(text(&written, &["checkpoint", "ref"]))),
            ("expected", Value::string("latest")),
        ],
    );
    assert_eq!(
        restored.get("restored_count").and_then(Value::as_usize),
        Some(1)
    );
    assert!(
        !fixture
            .tools
            .checkpoints()
            .unwrap()
            .take_restored_instructions()
    );
}

#[test]
fn checkpoint_schema_offers_only_call_recovery_actions() {
    let definitions = definitions();
    let checkpoint = definitions
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool.get("name") == Some(&Value::string("checkpoint")))
        .unwrap();
    let actions = checkpoint
        .get("parameters")
        .unwrap()
        .get("properties")
        .unwrap()
        .get("action")
        .unwrap()
        .get("enum")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        actions,
        &[
            Value::string("list"),
            Value::string("inspect"),
            Value::string("restore")
        ]
    );
}
