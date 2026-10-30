use super::*;
use crate::{
    effort::Effort,
    json::Value,
    redact::Redactor,
    test_support::{Directory, tool_call},
};
use std::{fs, io::Write};

mod attachment_tests;

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
fn tool_change_survives_journal_and_replays_without_entering_tool_result() {
    let (_directory, _home, store, mut doc) = fixture();
    let change = crate::file_change::Change::between(Some("old\n"), "new\n").unwrap();
    doc.messages.push(Value::object([
        ("role", Value::string("assistant")),
        (
            "tool_calls",
            Value::Array(vec![tool_call("call-1", "edit", Value::object([]))]),
        ),
    ]));
    doc.messages.push(Value::object([
        ("role", Value::string("tool")),
        ("tool_call_id", Value::string("call-1")),
        ("content", Value::string("{\"bytes_written\":4}")),
    ]));
    doc.events.push(Value::object([
        ("type", Value::string("tool_change")),
        ("after_message", Value::number(doc.messages.len())),
        ("tool_call_id", Value::string("call-1")),
        ("change", change.value()),
    ]));
    let lease = store.create(&doc.id).unwrap();
    lease.save(&doc).unwrap();
    drop(lease);
    let (handle, _) = Handle::open(store, &doc.id, Redactor::empty()).unwrap();
    let restored = handle.snapshot();
    assert_eq!(
        restored.messages[3].get("content").and_then(Value::as_str),
        Some("{\"bytes_written\":4}")
    );
    assert!(matches!(restored.records().last(),
        Some(Record::Tool { change: Some(actual), .. }) if actual == &change));
}

#[test]
fn routine_labels_and_descriptions_survive_journal_resume_and_record_projection() {
    let (_directory, _home, store, mut doc) = fixture();
    let read = Value::object([
        ("path", Value::string("src/lib.rs")),
        ("work", Value::string("Inspect the parser")),
        ("description", Value::string("Read the parser entry point")),
    ]);
    let bash = Value::object([
        ("command", Value::string("cargo test --locked --offline")),
        ("work", Value::string("Verify the parser")),
        ("description", Value::string("Run the parser tests")),
    ]);
    let calls = vec![
        tool_call("read-labeled", "read", read.clone()),
        crate::conversation::ToolCall::parse(&tool_call("bash-labeled", "bash", bash.clone()))
            .unwrap()
            .value(),
        tool_call(
            "read-legacy",
            "read",
            Value::object([("path", Value::string("README.md"))]),
        ),
    ];
    doc.messages.push(Value::object([
        ("role", Value::string("assistant")),
        ("tool_calls", Value::Array(calls)),
    ]));
    for id in ["read-labeled", "bash-labeled", "read-legacy"] {
        doc.messages.push(Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string(id)),
            ("content", Value::string("{}")),
        ]));
    }

    let lease = store.create(&doc.id).unwrap();
    lease.save(&doc).unwrap();
    drop(lease);
    let (handle, _) = Handle::open(store, &doc.id, Redactor::empty()).unwrap();
    let restored = handle.snapshot();
    assert_eq!(restored.messages, doc.messages);
    let tools: Vec<_> = restored
        .records()
        .into_iter()
        .filter_map(|record| match record {
            Record::Tool {
                id,
                name,
                arguments,
                ..
            } => Some((id, name, arguments)),
            _ => None,
        })
        .collect();
    assert_eq!(tools.len(), 3);
    assert_eq!(tools[0].0, "read-labeled");
    assert_eq!(tools[0].1, "read");
    assert_eq!(tools[0].2, read);
    assert_eq!(tools[1].0, "bash-labeled");
    assert_eq!(tools[1].1, "bash");
    assert_eq!(tools[1].2, bash);
    assert_eq!(tools[2].0, "read-legacy");
    assert_eq!(tools[2].1, "read");
    assert_eq!(
        tools[2].2,
        Value::object([("path", Value::string("README.md"))])
    );
    assert!(tools[2].2.get("work").is_none());
    assert!(tools[2].2.get("description").is_none());
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
fn incomplete_invalid_or_unsupported_first_record_is_rejected_unchanged() {
    for variant in 0..5 {
        let (_directory, _home, store, doc) = fixture();
        let record = match variant {
            0 => Vec::new(),
            1 => b"{\"sequence\":0,unfinished".to_vec(),
            2 => b"not json\n".to_vec(),
            3 => b"{\"format\":\"jecode.session.journal\",\"format_version\":4}\n".to_vec(),
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
        let journal = store.journal_path(&doc.id);
        fs::create_dir_all(journal.parent().unwrap()).unwrap();
        fs::write(&journal, &record).unwrap();
        let listing = store.list().unwrap();
        assert!(listing.sessions.is_empty());
        assert_eq!(listing.warnings.len(), 1);
        assert!(Handle::open(store, &doc.id, Redactor::empty()).is_err());
        assert_eq!(fs::read(&journal).unwrap(), record);
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

#[test]
fn a_saved_tail_appends_like_the_whole_history_and_a_gap_is_refused() {
    let (_directory, _home, store, mut doc) = fixture();
    let lease = store.create(&doc.id).unwrap();
    lease.save(&doc).unwrap();
    doc.messages.push(message("assistant", "First answer"));
    doc.events.push(Value::object([
        ("type", Value::string("note")),
        ("after_message", Value::number(3)),
    ]));
    let (messages, events) = lease.history().unwrap();
    assert_eq!((messages, events), (2, 0));
    let path = store.bucket().join(format!("{}.jsonl", doc.id));
    let before = fs::read(&path).unwrap();
    let state = doc.without_history();
    let gap = journal::Tail {
        document: &state,
        messages_from: 3,
        messages: &[],
        events_from: 1,
        events: &[],
    };
    assert!(lease.save_tail(&gap, &doc.title()).is_err());
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "a refused tail writes nothing"
    );
    let tail = journal::Tail {
        document: &state,
        messages_from: messages,
        messages: &doc.messages[messages..],
        events_from: events,
        events: &doc.events[events..],
    };
    lease.save_tail(&tail, &doc.title()).unwrap();
    assert_eq!(lease.history(), Some((3, 1)));
    drop(lease);
    let (handle, _) = Handle::open(store, &doc.id, Redactor::empty()).unwrap();
    let restored = handle.snapshot();
    assert_eq!(restored.messages, doc.messages);
    assert_eq!(restored.events, doc.events);
}
