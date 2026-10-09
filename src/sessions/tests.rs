use super::*;
use crate::{
    effort::Effort,
    json::{self, Value},
    redact::Redactor,
    test_support::{Directory, tool_call},
};
use std::fs;

fn message(role: &str, text: &str) -> Value {
    Value::object([
        ("role", Value::string(role)),
        ("content", Value::string(text)),
    ])
}
fn document(directory: &Directory) -> Document {
    let mut document = Document::new(
        directory.path().to_path_buf(),
        "fixture/model".into(),
        Effort::High,
    )
    .unwrap();
    document.messages = vec![
        message("system", "fixture system"),
        message("user", "First request"),
        message("assistant", "Answer"),
    ];
    document
}
fn store(home: &Directory, directory: &Directory) -> Store {
    Store::new(home.path().to_path_buf(), directory.path()).unwrap()
}

#[test]
fn folder_scope_explicit_load_and_exclusive_writer_survive_restart() {
    let home = Directory::new();
    let a = Directory::new();
    let b = Directory::new();
    let store_a = store(&home, &a);
    let store_b = store(&home, &b);
    let doc = document(&a);
    let lease = store_a.acquire(&doc.id).unwrap();
    lease.save(&doc).unwrap();
    assert_eq!(store_a.list().unwrap().sessions.len(), 1);
    assert!(store_b.list().unwrap().sessions.is_empty());
    assert!(store_b.load(&doc.id).is_err());
    assert!(store_a.acquire(&doc.id).is_err());
    drop(lease);
    let (handle, _) = Handle::open(
        store_a.clone(),
        &doc.id,
        Redactor::new("fixture-key".into()),
    )
    .unwrap();
    assert_eq!(handle.snapshot().messages, doc.messages);
    assert_eq!(handle.snapshot().effort, Effort::High);
    assert!(
        Handle::open(
            store_a.clone(),
            &doc.id,
            Redactor::new("fixture-key".into())
        )
        .is_err()
    );
    drop(handle);
    assert!(store_a.acquire(&doc.id).is_ok());
    assert!(store_a.load("../config").is_err());
}

#[cfg(windows)]
#[test]
fn canonical_folder_aliases_share_the_same_session_scope() {
    let home = Directory::new();
    let directory = Directory::new();
    let original = store(&home, &directory);
    let alias = std::path::PathBuf::from(directory.path().to_string_lossy().to_uppercase());
    let alias = Store::new(home.path().to_path_buf(), &alias).unwrap();
    let doc = document(&directory);
    original.acquire(&doc.id).unwrap().save(&doc).unwrap();
    assert_eq!(alias.list().unwrap().sessions.len(), 1);
    assert!(alias.load(&doc.id).is_ok());
}

#[test]
fn interrupted_tool_is_unknown_unstarted_calls_are_not_executed_and_partial_is_display_only() {
    let home = Directory::new();
    let directory = Directory::new();
    let store = store(&home, &directory);
    let mut doc = document(&directory);
    doc.messages.pop();
    doc.messages.push(Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::Null),
        (
            "tool_calls",
            Value::Array(vec![
                tool_call(
                    "started",
                    "write",
                    Value::object([
                        ("path", Value::string("a.txt")),
                        ("content", Value::string("must not execute")),
                    ]),
                ),
                tool_call(
                    "unstarted",
                    "bash",
                    Value::object([("command", Value::string("must not execute"))]),
                ),
            ]),
        ),
    ]));
    doc.pending = Pending {
        active: true,
        tool: Some("started".into()),
        partial: "incomplete response".into(),
    };
    let lease = store.acquire(&doc.id).unwrap();
    lease.save(&doc).unwrap();
    drop(lease);
    let (handle, status) =
        Handle::open(store, &doc.id, Redactor::new("fixture-key".into())).unwrap();
    let recovered = handle.snapshot();
    assert!(status.contains("Interrupted"));
    assert!(!directory.path().join("a.txt").exists());
    let unknown = json::parse(
        recovered.messages[3]
            .get("content")
            .unwrap()
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let unstarted = json::parse(
        recovered.messages[4]
            .get("content")
            .unwrap()
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        unknown.get("outcome").and_then(Value::as_str),
        Some("unknown")
    );
    assert_eq!(
        unstarted.get("outcome").and_then(Value::as_str),
        Some("not_executed")
    );
    assert!(
        !Value::Array(recovered.messages.clone())
            .encode()
            .contains("incomplete response")
    );
    assert!(recovered.records().iter().any(
        |record| matches!(record, Record::Text { text, .. } if text == "incomplete response")
    ));
    assert!(!recovered.pending.active);
    drop(handle);
}

#[test]
fn queue_and_withdrawn_edit_recover_in_order_without_losing_the_prior_draft() {
    let mut input = Input {
        staged: vec![],
        draft: Draft {
            text: "edited latest".into(),
            cursor: 3,
            ..Default::default()
        },
        queued: vec!["first queued".into(), "second queued".into()],
        paused: vec![Draft {
            text: "already paused".into(),
            cursor: 7,
            ..Default::default()
        }],
        previous: Some(Draft {
            text: "earlier draft".into(),
            cursor: 2,
            ..Default::default()
        }),
        history: vec!["last prompt".into(), "  /help".into(), "  prompt".into()],
    };
    let saved_before_recovery = input.clone();
    input.recover();
    assert_eq!(input.draft.text, "earlier draft");
    assert_eq!(input.draft.cursor, 2);
    assert_eq!(
        input.paused,
        [
            Draft {
                text: "first queued".into(),
                cursor: "first queued".len(),
                ..Default::default()
            },
            Draft {
                text: "second queued".into(),
                cursor: "second queued".len(),
                ..Default::default()
            },
            Draft {
                text: "already paused".into(),
                cursor: 7,
                ..Default::default()
            },
            Draft {
                text: "edited latest".into(),
                cursor: 3,
                ..Default::default()
            },
        ]
    );
    assert_eq!(input.history, ["last prompt", "  prompt"]);
    assert!(input.queued.is_empty());
    assert!(input.previous.is_none());
    let recovered = input.clone();
    input.recover();
    assert_eq!(input, recovered);
    let home = Directory::new();
    let directory = Directory::new();
    let store = store(&home, &directory);
    let mut doc = document(&directory);
    doc.input = saved_before_recovery;
    let lease = store.acquire(&doc.id).unwrap();
    lease.save(&doc).unwrap();
    drop(lease);
    let (handle, _) =
        Handle::open(store.clone(), &doc.id, Redactor::new("fixture-key".into())).unwrap();
    let saved = handle.snapshot();
    assert!(saved.input.queued.is_empty());
    assert!(saved.input.previous.is_none());
    assert_eq!(saved.input.draft, recovered.draft);
    assert_eq!(saved.input.paused, recovered.paused);
    assert_eq!(saved.input.history, ["last prompt", "  prompt"]);
    drop(handle);
    let (handle, _) = Handle::open(store, &doc.id, Redactor::new("fixture-key".into())).unwrap();
    assert_eq!(handle.snapshot().input.paused, recovered.paused);
}

#[test]
fn saved_paused_drafts_round_trip_and_legacy_snapshots_migrate() {
    let directory = Directory::new();
    let mut doc = document(&directory);
    doc.input.paused = vec![
        Draft {
            text: "same 🙂".into(),
            cursor: "same ".len(),
            ..Default::default()
        },
        Draft {
            text: "same 🙂".into(),
            cursor: "same 🙂".len(),
            ..Default::default()
        },
    ];
    assert_eq!(
        Document::parse(&doc.value()).unwrap().input.paused,
        doc.input.paused
    );

    let Value::Object(mut legacy) = doc.value() else {
        unreachable!()
    };
    let Value::Object(input) = legacy.get_mut("input").unwrap() else {
        unreachable!()
    };
    input.remove("paused");
    let mut parsed = Document::parse(&Value::Object(legacy)).unwrap();
    assert!(parsed.input.paused.is_empty());
    parsed.input.queued = vec!["same 🙂".into(), "same 🙂".into()];
    parsed.input.draft = Draft {
        text: "withdrawn".into(),
        cursor: 4,
        ..Default::default()
    };
    parsed.input.previous = Some(Draft {
        text: "original".into(),
        cursor: 1,
        ..Default::default()
    });
    parsed.recover();
    assert_eq!(parsed.input.draft.text, "original");
    assert_eq!(parsed.input.draft.cursor, 1);
    assert_eq!(
        parsed
            .input
            .paused
            .iter()
            .map(|draft| draft.text.as_str())
            .collect::<Vec<_>>(),
        ["same 🙂", "same 🙂", "withdrawn"]
    );
    assert_eq!(parsed.input.paused[2].cursor, 4);
}

#[test]
fn five_identical_pending_inputs_stay_separate_after_repeated_recovery() {
    let mut input = Input {
        draft: Draft {
            text: "repeat".into(),
            cursor: 2,
            ..Default::default()
        },
        queued: vec!["repeat".into(), "repeat".into(), "repeat".into()],
        paused: vec![Draft {
            text: "repeat".into(),
            cursor: 1,
            ..Default::default()
        }],
        ..Input::default()
    };
    input.recover();
    assert_eq!(input.paused.len(), 4);
    assert_eq!(input.draft.text, "repeat");
    assert!(input.paused.iter().all(|draft| draft.text == "repeat"));
    let once = input.clone();
    input.recover();
    assert_eq!(input, once);
}

#[test]
fn damaged_primary_recovers_backup_and_preserves_the_damaged_bytes() {
    let home = Directory::new();
    let directory = Directory::new();
    let store = store(&home, &directory);
    let mut doc = document(&directory);
    let lease = store.acquire(&doc.id).unwrap();
    lease.save_legacy(&doc).unwrap();
    doc.messages.push(message("user", "Later request"));
    lease.save_legacy(&doc).unwrap();
    drop(lease);
    let path = storage::file_path(&store, &doc.id);
    fs::write(&path, "{broken primary").unwrap();
    let (handle, status) =
        Handle::open(store.clone(), &doc.id, Redactor::new("fixture-key".into())).unwrap();
    assert!(status.contains("last valid"));
    assert_eq!(handle.snapshot().messages.len(), 3);
    assert_eq!(store.load(&doc.id).unwrap().0.messages.len(), 3);
    let damaged = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .flatten()
        .find(|entry| entry.file_name().to_string_lossy().contains("damaged"))
        .unwrap();
    assert_eq!(
        fs::read_to_string(damaged.path()).unwrap(),
        "{broken primary"
    );
}

#[test]
fn a_new_session_cannot_replace_an_existing_identifier() {
    let home = Directory::new();
    let directory = Directory::new();
    let store = store(&home, &directory);
    let doc = document(&directory);
    store.acquire(&doc.id).unwrap().save(&doc).unwrap();
    let mut conflicting = doc.clone();
    conflicting.messages[1] = message("user", "Must not replace saved work");
    let handle = Handle::new(
        store.clone(),
        conflicting,
        Redactor::new("fixture-key".into()),
    );
    assert!(handle.flush().unwrap_err().contains("already exists"));
    assert_eq!(store.load(&doc.id).unwrap().0.messages, doc.messages);
}

#[test]
fn a_future_format_is_not_downgraded_from_an_older_backup() {
    let home = Directory::new();
    let directory = Directory::new();
    let store = store(&home, &directory);
    let doc = document(&directory);
    let lease = store.acquire(&doc.id).unwrap();
    lease.save_legacy(&doc).unwrap();
    lease.save_legacy(&doc).unwrap();
    drop(lease);
    let path = storage::file_path(&store, &doc.id);
    let Value::Object(mut future) = doc.value() else {
        unreachable!()
    };
    future.insert("format_version".into(), Value::number(2));
    let future = Value::Object(future).pretty();
    fs::write(&path, &future).unwrap();
    let result = Handle::open(store.clone(), &doc.id, Redactor::new("fixture-key".into()));
    assert!(matches!(result, Err(error) if error.contains("Unsupported session")));
    assert_eq!(fs::read_to_string(path).unwrap(), future);
    assert!(store.list().unwrap().sessions.is_empty());
    assert_eq!(store.list().unwrap().warnings.len(), 1);
}

#[test]
fn invalid_context_cursor_scope_and_future_format_are_rejected() {
    let directory = Directory::new();
    let doc = document(&directory);
    for (field, value) in [
        ("format_version", Value::number(2)),
        ("compatible_from", Value::number(999)),
        ("id", Value::string("../bad")),
    ] {
        let Value::Object(mut value_doc) = doc.value() else {
            unreachable!()
        };
        value_doc.insert(field.into(), value);
        assert!(Document::parse(&Value::Object(value_doc)).is_err());
    }
    let mut invalid = doc.clone();
    invalid.input.draft = Draft {
        text: "è".into(),
        cursor: 1,
        ..Default::default()
    };
    assert!(Document::parse(&invalid.value()).is_err());
    invalid = doc.clone();
    invalid.input.paused.push(Draft {
        text: "è".into(),
        cursor: 1,
        ..Default::default()
    });
    assert!(Document::parse(&invalid.value()).is_err());
    invalid = doc.clone();
    invalid.messages.push(Value::object([
        ("role", Value::string("tool")),
        ("tool_call_id", Value::string("unpaired")),
        ("content", Value::string("{}")),
    ]));
    assert!(Document::parse(&invalid.value()).is_err());
}

#[test]
fn input_redaction_keeps_utf8_cursor_valid_and_never_stores_the_key() {
    let home = Directory::new();
    let directory = Directory::new();
    let store = store(&home, &directory);
    let key = "fixture-secret-long-key";
    let doc = document(&directory);
    let id = doc.id.clone();
    let handle = Handle::new(store.clone(), doc, Redactor::new(key.into()));
    let text = format!("{key} è draft");
    let cursor = format!("{key} è").len();
    handle.input(Input {
        draft: Draft {
            text,
            cursor,
            ..Default::default()
        },
        ..Input::default()
    });
    handle.flush().unwrap();
    let saved = fs::read_to_string(storage::file_path(&store, &id)).unwrap();
    assert!(!saved.contains(key));
    let doc = store.fixture_load(&id).unwrap();
    assert_eq!(
        &doc.input.draft.text[..doc.input.draft.cursor],
        "[redacted] è"
    );
    let text = format!("è {key} after");
    handle.input(Input {
        draft: Draft {
            text,
            cursor: "è ".len() + 15,
            ..Default::default()
        },
        ..Input::default()
    });
    handle.flush().unwrap();
    let doc = store.fixture_load(&id).unwrap();
    assert_eq!(
        &doc.input.draft.text[..doc.input.draft.cursor],
        "è [redacted]"
    );
    handle.input(Input {
        paused: vec![Draft {
            text: format!("before {key} after"),
            cursor: format!("before {key}").len(),
            ..Default::default()
        }],
        ..Input::default()
    });
    handle.flush().unwrap();
    let saved = fs::read_to_string(storage::file_path(&store, &id)).unwrap();
    assert!(!saved.contains(key));
    let doc = store.fixture_load(&id).unwrap();
    assert_eq!(
        &doc.input.paused[0].text[..doc.input.paused[0].cursor],
        "before [redacted]"
    );
}
