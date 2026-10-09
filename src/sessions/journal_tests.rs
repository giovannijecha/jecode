use super::*;
use crate::{effort::Effort, json::Value, redact::Redactor, test_support::Directory};
use std::{fs, io::Write};

fn message(role: &str, text: &str) -> Value {
    Value::object([
        ("role", Value::string(role)),
        ("content", Value::string(text)),
    ])
}

fn fixture() -> (Directory, Directory, Store, Document) {
    let directory = Directory::new();
    let home = Directory::new();
    let store = Store::new(home.path().to_path_buf(), directory.path()).unwrap();
    let mut doc = Document::new(
        directory.path().to_path_buf(),
        "fixture/model".into(),
        Effort::Default,
    )
    .unwrap();
    doc.messages = vec![
        message("system", "Fixture"),
        message("user", "Original request"),
    ];
    (directory, home, store, doc)
}

#[test]
fn long_history_passes_the_previous_save_limit_and_each_checkpoint_appends_only_new_data() {
    let (_directory, _home, store, mut doc) = fixture();
    let lease = store.create(&doc.id).unwrap();
    let text = "x".repeat(1024 * 1024);
    let path = store.journal_path(&doc.id);
    for _ in 0..65 {
        let previous = fs::metadata(&path).map_or(0, |metadata| metadata.len());
        doc.messages.push(message("assistant", &text));
        lease.save(&doc).unwrap();
        let added = fs::metadata(&path).unwrap().len() - previous;
        assert!(added < text.len() as u64 + 4096);
    }
    assert!(fs::metadata(&path).unwrap().len() > 64 * 1024 * 1024);
    drop(lease);
    let loaded = store.fixture_load(&doc.id).unwrap();
    assert_eq!(loaded.messages.len(), 67);
    assert_eq!(loaded.messages.last(), doc.messages.last());
}

#[test]
fn a_torn_final_transaction_recovers_prior_work_and_preserves_the_uncommitted_bytes() {
    let (_directory, _home, store, mut doc) = fixture();
    let lease = store.create(&doc.id).unwrap();
    lease.save(&doc).unwrap();
    doc.messages.push(message("assistant", "Already finished"));
    lease.save(&doc).unwrap();
    drop(lease);
    let path = store.journal_path(&doc.id);
    let torn = b"{\"sequence\":2,\"messages\":[unfinished";
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(torn)
        .unwrap();
    let (handle, status) =
        Handle::open(store.clone(), &doc.id, Redactor::new("fixture-key".into())).unwrap();
    assert!(status.contains("last valid checkpoint"));
    assert_eq!(handle.snapshot().messages, doc.messages);
    assert_eq!(store.fixture_load(&doc.id).unwrap().messages, doc.messages);
    let damaged = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .flatten()
        .find(|entry| entry.file_name().to_string_lossy().contains("damaged"))
        .unwrap();
    assert_eq!(fs::read(damaged.path()).unwrap(), torn);
}

#[test]
fn legacy_sessions_migrate_on_resume_without_replacing_the_original_file() {
    let (_directory, _home, store, doc) = fixture();
    store.acquire(&doc.id).unwrap().save_legacy(&doc).unwrap();
    let path = storage::file_path(&store, &doc.id);
    let original = fs::read(&path).unwrap();
    let (handle, _) =
        Handle::open(store.clone(), &doc.id, Redactor::new("fixture-key".into())).unwrap();
    assert_eq!(handle.snapshot().messages, doc.messages);
    assert_eq!(fs::read(path).unwrap(), original);
    assert!(store.journal_path(&doc.id).exists());
    assert_eq!(store.list().unwrap().sessions.len(), 1);
}

#[test]
fn empty_or_torn_first_journal_restores_legacy_without_listing_mutations() {
    for initial in [b"".as_slice(), b"{\"sequence\":0,unfinished".as_slice()] {
        let (_directory, _home, store, doc) = fixture();
        store.acquire(&doc.id).unwrap().save_legacy(&doc).unwrap();
        let legacy = storage::file_path(&store, &doc.id);
        let original = fs::read(&legacy).unwrap();
        let journal = store.journal_path(&doc.id);
        fs::write(&journal, initial).unwrap();

        let listing = store.list().unwrap();
        assert_eq!(listing.sessions.len(), 1);
        assert!(listing.warnings.is_empty());
        assert_eq!(fs::read(&journal).unwrap(), initial);
        assert_eq!(fs::read(&legacy).unwrap(), original);
        assert!(
            !store
                .bucket
                .join(format!("{}.summary.json", doc.id))
                .exists()
        );

        let (handle, _) = Handle::open(store.clone(), &doc.id, Redactor::empty()).unwrap();
        assert_eq!(handle.snapshot().messages, doc.messages);
        assert_eq!(fs::read(&legacy).unwrap(), original);
        assert_eq!(store.fixture_load(&doc.id).unwrap().messages, doc.messages);
        let damaged = fs::read_dir(journal.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("jsonl.damaged-")
            })
            .collect::<Vec<_>>();
        assert_eq!(damaged.len(), usize::from(!initial.is_empty()));
        if let Some(damaged) = damaged.first() {
            assert_eq!(fs::read(damaged.path()).unwrap(), initial);
        }
        drop(handle);
        let (reopened, _) = Handle::open(store, &doc.id, Redactor::empty()).unwrap();
        assert_eq!(reopened.snapshot().messages, doc.messages);
    }
}

#[test]
fn torn_first_journal_can_restore_a_valid_legacy_backup() {
    let (_directory, _home, store, mut doc) = fixture();
    let lease = store.acquire(&doc.id).unwrap();
    lease.save_legacy(&doc).unwrap();
    let legacy = storage::file_path(&store, &doc.id);
    let original = fs::read(&legacy).unwrap();
    doc.messages.push(message("assistant", "Later version"));
    lease.save_legacy(&doc).unwrap();
    drop(lease);
    fs::write(&legacy, b"damaged primary").unwrap();
    let backup = legacy.with_extension("json.bak");
    assert_eq!(fs::read(&backup).unwrap(), original);
    let torn = b"incomplete first checkpoint";
    let journal = store.journal_path(&doc.id);
    fs::write(&journal, torn).unwrap();
    assert_eq!(store.list().unwrap().sessions.len(), 1);
    assert_eq!(fs::read(&journal).unwrap(), torn);
    let (handle, _) = Handle::open(store.clone(), &doc.id, Redactor::empty()).unwrap();
    assert_eq!(handle.snapshot().messages.len(), 2);
    assert_eq!(fs::read(&legacy).unwrap(), b"damaged primary");
    assert_eq!(fs::read(&backup).unwrap(), original);
    assert_eq!(store.fixture_load(&doc.id).unwrap().messages.len(), 2);
}

#[test]
fn complete_invalid_or_future_first_record_never_falls_back_to_legacy() {
    for variant in 0..3 {
        let (_directory, _home, store, doc) = fixture();
        let record = match variant {
            0 => b"not json\n".to_vec(),
            1 => b"{\"format\":\"jecode.session.journal\",\"format_version\":4}\n".to_vec(),
            _ => {
                let mut state = doc.state_value();
                let Value::Object(fields) = &mut state else {
                    unreachable!()
                };
                fields.insert("id".into(), Value::string("123-456-789"));
                format!(
                    "{}\n",
                    Value::object([
                        ("format", Value::string("jecode.session.journal")),
                        ("format_version", Value::number(2)),
                        ("sequence", Value::number(0)),
                        ("messages_from", Value::number(0)),
                        ("events_from", Value::number(0)),
                        ("messages", Value::Array(doc.messages.clone())),
                        ("events", Value::Array(vec![])),
                        ("state", state),
                    ])
                    .encode()
                )
                .into_bytes()
            }
        };
        store.acquire(&doc.id).unwrap().save_legacy(&doc).unwrap();
        let legacy = storage::file_path(&store, &doc.id);
        let original = fs::read(&legacy).unwrap();
        let journal = store.journal_path(&doc.id);
        fs::write(&journal, &record).unwrap();
        let listing = store.list().unwrap();
        assert!(listing.sessions.is_empty());
        assert_eq!(listing.warnings.len(), 1);
        assert!(Handle::open(store, &doc.id, Redactor::empty()).is_err());
        assert_eq!(fs::read(&journal).unwrap(), record);
        assert_eq!(fs::read(&legacy).unwrap(), original);
    }
}

#[test]
fn streaming_checkpoints_grow_linearly_and_recover_the_last_complete_utf8_prefix() {
    let (_directory, _home, store, mut doc) = fixture();
    doc.pending.active = true;
    doc.context.summary = "An unchanged continuity summary".repeat(1024);
    doc.input.history = vec!["An unchanged prompt".repeat(1024).into()];
    let id = doc.id.clone();
    let handle = Handle::new(store.clone(), doc, Redactor::empty());
    handle.flush().unwrap();
    let text = "🦀".repeat(64 * 1024);
    for end in (4096..=text.len()).step_by(4096) {
        handle.partial(&text[..end]).unwrap();
        handle.flush().unwrap();
    }
    let path = store.journal_path(&id);
    assert!(fs::metadata(&path).unwrap().len() < text.len() as u64 * 2);
    assert_eq!(store.fixture_load(&id).unwrap().pending.partial, text);
    drop(handle);
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"format_version\":3,\"partial\":unfinished")
        .unwrap();
    let (recovered, status) = Handle::open(store, &id, Redactor::empty()).unwrap();
    assert!(status.contains("last valid checkpoint"));
    let doc = recovered.snapshot();
    assert!(!doc.pending.active);
    assert!(
        doc.events
            .iter()
            .any(|event| event.get("partial_text").and_then(Value::as_str) == Some(text.as_str()))
    );
}

#[test]
fn streaming_resets_and_replacements_do_not_mix_attempts_or_redacted_prefixes() {
    let (_directory, _home, store, mut doc) = fixture();
    doc.pending.active = true;
    let id = doc.id.clone();
    let handle = Handle::new(store.clone(), doc, Redactor::new("fixture-secret".into()));
    for text in [
        "First 🦀 attempt",
        "",
        "Second 🦀 fixture-",
        "Second 🦀 fixture-secret",
        "Final 🦀",
    ] {
        handle.partial(text).unwrap();
        handle.flush().unwrap();
        assert_eq!(
            store.fixture_load(&id).unwrap().pending.partial,
            Redactor::new("fixture-secret".into()).text(text)
        );
    }
    assert!(
        !fs::read_to_string(store.journal_path(&id))
            .unwrap()
            .contains("fixture-secret")
    );
}

#[test]
fn a_completed_stream_keeps_the_final_message_and_has_bounded_journal_growth() {
    let (_directory, _home, store, mut doc) = fixture();
    doc.pending.active = true;
    let lease = store.create(&doc.id).unwrap();
    let text = "x".repeat(256 * 1024);
    for end in (4096..=text.len()).step_by(4096) {
        doc.pending.partial = text[..end].into();
        lease.save(&doc).unwrap();
    }
    doc.messages.push(message("assistant", &text));
    doc.pending = Pending::default();
    lease.save(&doc).unwrap();
    let saved = store.fixture_load(&doc.id).unwrap();
    assert_eq!(
        saved
            .messages
            .last()
            .unwrap()
            .get("content")
            .and_then(Value::as_str),
        Some(text.as_str())
    );
    assert!(saved.pending.partial.is_empty());
    assert!(!saved.pending.active);
    let bytes = fs::metadata(store.journal_path(&doc.id)).unwrap().len();
    assert!(
        bytes < text.len() as u64 * 3,
        "Completed journal contains {bytes} bytes"
    );
}

#[test]
fn version_two_journals_accept_new_deltas_without_losing_their_state() {
    let (_directory, _home, store, mut doc) = fixture();
    doc.pending.active = true;
    doc.pending.partial = "Original 🦀".into();
    let lease = store.acquire(&doc.id).unwrap();
    let record = Value::object([
        ("format", Value::string("jecode.session.journal")),
        ("format_version", Value::number(2)),
        ("sequence", Value::number(0)),
        ("messages_from", Value::number(0)),
        ("events_from", Value::number(0)),
        ("messages", Value::Array(doc.messages.clone())),
        ("events", Value::Array(doc.events.clone())),
        ("state", doc.state_value()),
    ]);
    fs::write(
        store.journal_path(&doc.id),
        format!("{}\n", record.encode()),
    )
    .unwrap();
    doc.pending.partial.push_str(" continued");
    doc.input.draft = Draft {
        text: "Unsent work".into(),
        cursor: 6,
        ..Default::default()
    };
    lease.save(&doc).unwrap();
    let saved = store.fixture_load(&doc.id).unwrap();
    assert_eq!(saved.pending.partial, doc.pending.partial);
    assert!(saved.input == doc.input);
    assert_eq!(saved.messages, doc.messages);
}

#[test]
fn invalid_delta_boundaries_and_future_journals_are_left_unchanged() {
    let (_directory, _home, store, mut doc) = fixture();
    doc.pending.active = true;
    doc.pending.partial = "🦀".into();
    let lease = store.acquire(&doc.id).unwrap();
    lease.save(&doc).unwrap();
    drop(lease);
    let path = store.journal_path(&doc.id);
    let original = fs::read_to_string(&path).unwrap();
    for (version, keep) in [(3, 1), (3, 99), (4, 0)] {
        let record = Value::object([
            ("format", Value::string("jecode.session.journal")),
            ("format_version", Value::number(version)),
            ("sequence", Value::number(1)),
            ("messages_from", Value::number(doc.messages.len())),
            ("events_from", Value::number(doc.events.len())),
            ("messages", Value::Array(vec![])),
            ("events", Value::Array(vec![])),
            ("state", Value::object([])),
            (
                "partial",
                Value::object([
                    ("keep_bytes", Value::number(keep)),
                    ("text", Value::string("bad")),
                ]),
            ),
        ]);
        let damaged = format!("{original}{}\n", record.encode());
        fs::write(&path, &damaged).unwrap();
        assert!(Handle::open(store.clone(), &doc.id, Redactor::empty()).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), damaged);
    }
}
