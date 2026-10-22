//! Declared-scope result names and cancellation during final observation.
use super::*;
use std::sync::{Arc, mpsc};

#[test]
fn bash_changed_names_stay_within_its_declared_scope() {
    let fixture = fixture();
    let arguments = Value::object([
        ("command", Value::string("printf second > notes.txt")),
        ("protect", Value::Array(vec![Value::string("notes.txt")])),
    ])
    .encode();
    let begun = fixture.tools.begin("bash", &arguments);

    let mut other = Tools::new(fixture.root.path()).unwrap();
    other.configure_output(
        fixture._home.path().join("other-logs"),
        crate::redact::Redactor::empty(),
    );
    let session = Session::offline(fixture._home.path(), other.root());
    session.begin("conversation-b", None);
    other.configure_checkpoints(session);
    let unrelated = call(
        &other,
        "write",
        &[
            ("path", Value::string("unrelated.txt")),
            ("content", Value::string("other session\n")),
        ],
    );
    assert_eq!(text(&unrelated, &["checkpoint", "status"]), "saved");

    let result = fixture
        .tools
        .execute_begun(begun, "bash", &arguments, &Cancellation::default())
        .result;
    let checkpoint = result.get("checkpoint").unwrap();
    assert_eq!(
        checkpoint.get("changed"),
        Some(&Value::Array(vec![Value::string("notes.txt")])),
        "{result:?}"
    );
}

#[test]
fn directory_changed_names_pass_similarly_prefixed_sibling() {
    let fixture = fixture();
    fs::create_dir(fixture.root.path().join("src")).unwrap();
    fs::write(fixture.root.path().join("src/main.rs"), "before\n").unwrap();
    let arguments = Value::object([
        ("command", Value::string("printf after > src/main.rs")),
        ("protect", Value::Array(vec![Value::string("src/")])),
    ])
    .encode();
    let begun = fixture.tools.begin("bash", &arguments);

    let mut other = Tools::new(fixture.root.path()).unwrap();
    other.configure_output(
        fixture._home.path().join("sibling-logs"),
        crate::redact::Redactor::empty(),
    );
    let session = Session::offline(fixture._home.path(), other.root());
    session.begin("conversation-b", None);
    other.configure_checkpoints(session);
    let unrelated = call(
        &other,
        "write",
        &[
            ("path", Value::string("src-other.txt")),
            ("content", Value::string("another session\n")),
        ],
    );
    assert_eq!(text(&unrelated, &["checkpoint", "status"]), "saved");

    let result = fixture
        .tools
        .execute_begun(begun, "bash", &arguments, &Cancellation::default())
        .result;
    assert_eq!(
        result.get("checkpoint").unwrap().get("changed"),
        Some(&Value::Array(vec![Value::string("src/main.rs")])),
        "{result:?}"
    );
}

#[test]
fn cancelled_bash_does_not_wait_for_postcapture_store_lock() {
    let fixture = fixture();
    let session = Arc::clone(fixture.tools.checkpoints().unwrap());
    let cancellation = Cancellation::default();
    let arguments = Value::object([
        ("command", Value::string("printf changed > notes.txt")),
        ("protect", Value::Array(vec![Value::string("notes.txt")])),
    ])
    .encode();
    let begun = fixture
        .tools
        .begin_cancellable("bash", &arguments, &cancellation);
    let tools = Arc::new(fixture.tools);
    let store = session.workspace().unwrap().test_hold_store();
    cancellation.cancel();

    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = tools
            .execute_begun(begun, "bash", &arguments, &cancellation)
            .result;
        let _ = tx.send(result);
    });
    let prompt = rx.recv_timeout(Duration::from_secs(1));
    let was_prompt = prompt.is_ok();
    drop(store);
    let result = match prompt {
        Ok(result) => result,
        Err(_) => rx.recv_timeout(Duration::from_secs(10)).unwrap(),
    };
    worker.join().unwrap();
    assert!(
        was_prompt,
        "cancelled Bash waited for the archive lock during finalization: {result:?}"
    );
}
