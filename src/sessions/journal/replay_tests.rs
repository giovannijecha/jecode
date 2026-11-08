//! Offline checks of saved context and sustained record growth. No backend is used.

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
    max_record_level: u8,
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
        self.max_record_level = self.max_record_level.max(
            context
                .digests
                .iter()
                .map(|span| span.level)
                .max()
                .unwrap_or(0),
        );
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
                    "max_record_level",
                    Value::number(self.max_record_level as usize)
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
            && checkpoint.get("format_version").and_then(Value::as_usize) == Some(3)
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
        state::merge(&mut saved, patch);
        state::clear_legacy_draft_attachments(&mut saved, patch);
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
    doc.context.push_digest(end, String::new());
    super::append(&path, &super::Tail::whole(&doc), &mut position).unwrap();
    let stats = replay(&path).unwrap();
    assert_eq!(stats.checkpoints, 2);
    assert_eq!(stats.max_record_level, 1);
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

#[test]
#[ignore = "synthetic record endurance; no provider or real elapsed-hour claim"]
fn offline_record_endurance() {
    const TURNS: usize = 360;
    const BUDGET: usize = 32_000;
    let started = Instant::now();
    let mut context = Context::default();
    let mut messages = vec![
        message("system", "Offline fixture"),
        message("user", "Turn 0"),
    ];
    let mut originals = Vec::new();
    let mut stats = Replay::default();
    for turn in 0..TURNS {
        let id = format!("read-{turn}");
        let path = format!("src/file{turn}.rs");
        messages.push(Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string("")),
            (
                "tool_calls",
                Value::Array(vec![crate::test_support::tool_call(
                    &id,
                    "read",
                    Value::object([("path", Value::string(path))]),
                )]),
            ),
        ]));
        let marker = format!("ARCHIVED_PAYLOAD_{turn}");
        let result = Value::object([
            (
                "content",
                Value::string(format!("{marker}\n{}", "x".repeat(64 * 1024))),
            ),
            ("lines_returned", Value::number(2)),
            ("total_lines", Value::number(2)),
        ])
        .encode();
        originals.push((messages.len(), marker));
        messages.push(Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string(id)),
            ("content", Value::string(result)),
        ]));
        messages.push(message("assistant", &format!("Finished turn {turn}")));
        let end = messages.len();
        let live = message("user", &format!("Turn {}", turn + 1));
        messages.push(live.clone());
        context.push_digest(end, String::new());
        context.roll_up(&messages, BUDGET);
        stats.check(&context, &messages, 0).unwrap();
        let projected = context.project(&messages, 0);
        assert_eq!(projected.last(), Some(&live), "the next request stays live");
        assert_eq!(
            projected
                .iter()
                .filter(|m| m.get(crate::context::digest::ANCHOR).is_some())
                .count(),
            1
        );
        let bytes = projected.iter().map(|message| message.encode().len()).sum();
        assert!(
            context.estimate_bytes(bytes) < BUDGET,
            "synthetic context exceeded its budget"
        );
    }
    for (index, marker) in originals {
        assert!(
            messages[index]
                .get("content")
                .and_then(Value::as_str)
                .unwrap()
                .contains(&marker)
        );
    }
    assert_eq!(stats.max_record_level, 3, "exercise both rollup levels");
    stats.report("synthetic_record_endurance_360_turns", started);
}
