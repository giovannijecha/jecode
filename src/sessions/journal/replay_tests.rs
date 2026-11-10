//! Offline checks of saved context. No backend is used.

use super::{Document, state};
use crate::{context::Context, json::Value};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::time::Instant;

#[derive(Default, Debug)]
struct Replay {
    checkpoints: usize,
    messages: usize,
    peak_projection_bytes: usize,
}

impl Replay {
    fn check(
        &mut self,
        context: &Context,
        messages: &[Value],
        compatible: usize,
    ) -> Result<(), String> {
        context.validate(messages)?;
        let before = context.project(messages, compatible);
        let mut restored = Context::parse(Some(&context.value()))?;
        restored.returned.clone_from(&context.returned);
        restored.validate(messages)?;
        if restored.project(messages, compatible) != before {
            return Err("Context projection changed across a save/resume round trip".into());
        }
        let bytes = before.iter().map(|message| message.encode().len()).sum();
        self.peak_projection_bytes = self.peak_projection_bytes.max(bytes);
        self.checkpoints += 1;
        self.messages = messages.len();
        Ok(())
    }

    fn report(&self, mode: &str, started: Instant) {
        eprintln!(
            "{}",
            Value::object([
                ("mode", Value::string(mode)),
                ("checkpoints", Value::number(self.checkpoints)),
                ("messages", Value::number(self.messages)),
                (
                    "peak_projection_bytes",
                    Value::number(self.peak_projection_bytes)
                ),
                (
                    "elapsed_ms",
                    Value::number(started.elapsed().as_millis() as usize)
                ),
                ("provider_requests", Value::number(0)),
            ])
            .encode()
        );
    }
}

fn replay(path: &Path) -> Result<Replay, String> {
    let mut reader = BufReader::new(File::open(path).map_err(|error| error.to_string())?);
    let mut saved = Value::object([]);
    let mut partial = String::new();
    let mut document: Option<Document> = None;
    let mut stats = Replay::default();
    loop {
        let mut line = Vec::new();
        if reader
            .read_until(b'\n', &mut line)
            .map_err(|error| error.to_string())?
            == 0
        {
            break;
        }
        if line.last() != Some(&b'\n') {
            return Err("Uncommitted checkpoint tail; source left unchanged".into());
        }
        let checkpoint = crate::json::parse(
            std::str::from_utf8(&line).map_err(|_| "Invalid checkpoint UTF-8")?,
        )?;
        let (mut messages, mut events) = document
            .take()
            .map_or((vec![], vec![]), |doc| (doc.messages, doc.events));
        let valid = checkpoint.get("format").and_then(Value::as_str)
            == Some("jecode.session.journal")
            && checkpoint.get("format_version").and_then(Value::as_usize) == Some(4)
            && checkpoint.get("sequence").and_then(Value::as_usize) == Some(stats.checkpoints)
            && checkpoint.get("messages_from").and_then(Value::as_usize) == Some(messages.len())
            && checkpoint.get("events_from").and_then(Value::as_usize) == Some(events.len());
        if !valid {
            return Err(format!(
                "Invalid continuation at checkpoint {}",
                stats.checkpoints
            ));
        }
        messages.extend_from_slice(super::array(&checkpoint, "messages")?);
        events.extend_from_slice(super::array(&checkpoint, "events")?);
        let patch = super::required(&checkpoint, "state")?;
        state::apply(&mut saved, patch);
        state::apply_partial(&mut partial, super::required(&checkpoint, "partial")?)?;
        let next = Document::parse_state(
            &state::with_partial(saved.clone(), &partial)?,
            messages,
            events,
        )?;
        stats.check(&next.context, &next.messages, next.compatible_from)?;
        document = Some(next);
    }
    if stats.checkpoints == 0 {
        return Err("No saved checkpoints to replay".into());
    }
    Ok(stats)
}

#[test]
#[ignore = "read-only replay; set JECODE_REPLAY_JOURNALS to explicit journal paths"]
fn offline_saved_context_replay() {
    let paths = std::env::var_os("JECODE_REPLAY_JOURNALS")
        .expect("Set JECODE_REPLAY_JOURNALS to saved .jsonl files (platform path separator)");
    let paths = std::env::split_paths(&paths).collect::<Vec<_>>();
    assert!(!paths.is_empty(), "At least one journal is required");
    for path in paths {
        let started = Instant::now();
        replay(&path)
            .unwrap_or_else(|error| panic!("Replay failed: {error}"))
            .report("saved_context_replay", started);
    }
}

fn message(role: &str, text: &str) -> Value {
    Value::object([
        ("role", Value::string(role)),
        ("content", Value::string(text)),
    ])
}

#[test]
fn previous_session_formats_are_rejected_without_changing_the_journal() {
    use crate::{effort::Effort, test_support::Directory};

    let directory = Directory::new();
    let path = directory.path().join("previous.jsonl");
    let mut doc = Document::new(
        directory.path().into(),
        "fixture/model".into(),
        Effort::Default,
    )
    .unwrap();
    doc.messages = vec![message("system", "Fixture"), message("user", "Task")];
    let mut position = None;
    super::append(&path, &super::Tail::whole(&doc), &mut position).unwrap();
    let current = std::fs::read_to_string(&path).unwrap();
    let checkpoint = crate::json::parse(&current).unwrap();

    for old_document in [false, true] {
        let mut previous = checkpoint.clone();
        let Value::Object(fields) = &mut previous else {
            unreachable!()
        };
        if old_document {
            let Some(Value::Object(state)) = fields.get_mut("state") else {
                unreachable!()
            };
            state.insert("format_version".into(), Value::number(1));
        } else {
            fields.insert("format_version".into(), Value::number(3));
        }
        let bytes = format!("{}\n", previous.encode());
        std::fs::write(&path, &bytes).unwrap();
        let error = match super::read(&path, &doc.id, &doc.directory) {
            Err(error) => error,
            Ok(_) => panic!("previous session format was accepted"),
        };
        assert!(
            error.contains("Unsupported session format or version"),
            "{error}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), bytes);
    }
}

#[test]
fn replay_checks_state_patches_and_rejects_broken_continuations() {
    use crate::{effort::Effort, test_support::Directory};
    let directory = Directory::new();
    let path = directory.path().join("replay.jsonl");
    let mut doc = Document::new(
        directory.path().into(),
        "fixture/model".into(),
        Effort::Default,
    )
    .unwrap();
    doc.messages = vec![message("system", "Fixture"), message("user", "First")];
    let mut position = None;
    super::append(&path, &super::Tail::whole(&doc), &mut position).unwrap();
    doc.messages.push(message("assistant", "First answer"));
    let end = doc.messages.len();
    doc.messages.push(message("user", "Second"));
    doc.context.from = end;
    doc.context.summary = "First exchange".into();
    super::append(&path, &super::Tail::whole(&doc), &mut position).unwrap();
    let stats = replay(&path).unwrap();
    assert_eq!(stats.checkpoints, 2);
    assert_eq!(stats.messages, 4);
    let mut lines = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, lines.trim_end_matches('\n')).unwrap();
    assert!(
        replay(&path)
            .unwrap_err()
            .contains("Uncommitted checkpoint tail")
    );
    let last = lines.rfind("\"sequence\":1").unwrap();
    lines.replace_range(last..last + "\"sequence\":1".len(), "\"sequence\":9");
    std::fs::write(&path, lines).unwrap();
    assert!(replay(&path).unwrap_err().contains("Invalid continuation"));
}
