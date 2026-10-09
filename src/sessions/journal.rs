use super::{
    Document, context,
    document::{array, integer, required},
    identifier, same_directory,
    storage::{options, reject_special},
};
use crate::json::{self, Value};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::Path;

mod state;

#[derive(Default)]
pub(super) struct Position {
    bytes: u64,
    messages: usize,
    events: usize,
    sequence: usize,
    state: Option<Value>,
    partial: String,
}

pub(super) struct Saved {
    pub document: Document,
    pub damaged: bool,
    position: Position,
}

pub(super) enum Read {
    Saved(Box<Saved>),
    NoCheckpoint,
}

pub(super) fn read(path: &Path, id: &str, directory: &Path) -> Result<Read, String> {
    reject_special(path)?;
    let mut reader = BufReader::new(
        File::open(path).map_err(|error| format!("Could not read session journal: {error}"))?,
    );
    let mut document: Option<Document> = None;
    let mut position = Position::default();
    let mut damaged = false;
    let mut checked_directory = None;
    loop {
        let mut line = Vec::new();
        let length = reader
            .read_until(b'\n', &mut line)
            .map_err(|error| error.to_string())?;
        if length == 0 {
            break;
        }
        if line.last() != Some(&b'\n') {
            damaged = true;
            break;
        }
        let value =
            json::parse(std::str::from_utf8(&line).map_err(|_| "Invalid session journal UTF-8")?)?;
        let version = integer(&value, "format_version")?;
        if value.get("format").and_then(Value::as_str) != Some("jecode.session.journal")
            || !matches!(version, 2 | 3)
        {
            return Err("Unsupported session format or version; file left unchanged".into());
        }
        if integer(&value, "sequence")? != position.sequence as u64
            || integer(&value, "messages_from")? != position.messages as u64
            || integer(&value, "events_from")? != position.events as u64
        {
            return Err("Session journal has an invalid continuation; file left unchanged".into());
        }
        let (mut messages, mut events) = document
            .take()
            .map_or((vec![], vec![]), |doc| (doc.messages, doc.events));
        messages.extend_from_slice(array(&value, "messages")?);
        events.extend_from_slice(array(&value, "events")?);
        let next_state = if version == 2 {
            let state = required(&value, "state")?;
            position.partial = state
                .get("pending")
                .and_then(|pending| pending.get("partial_text"))
                .and_then(Value::as_str)
                .ok_or("Missing journal partial response")?
                .into();
            state::without_partial(state.clone())
        } else {
            let mut next = position.state.take().unwrap_or_else(|| Value::object([]));
            let patch = required(&value, "state")?;
            if !matches!(patch, Value::Object(_)) {
                return Err("Invalid journal state patch".into());
            }
            state::merge(&mut next, patch);
            state::apply_partial(&mut position.partial, required(&value, "partial")?)?;
            next
        };
        let next = Document::parse_state(
            &state::with_partial(next_state.clone(), &position.partial)?,
            messages,
            events,
        )?;
        if next.id != id
            || checked_directory.as_ref() != Some(&next.directory)
                && !same_directory(&next.directory, directory)
        {
            return Err(
                "Session identifier or working directory differs; file left unchanged".into(),
            );
        }
        checked_directory = Some(next.directory.clone());
        position.bytes += length as u64;
        position.messages = next.messages.len();
        position.events = next.events.len();
        position.sequence += 1;
        position.state = Some(next_state);
        document = Some(next);
    }
    let Some(document) = document else {
        return Ok(Read::NoCheckpoint);
    };
    context::validate(&document)?;
    Ok(Read::Saved(Box::new(Saved {
        document,
        damaged,
        position,
    })))
}

pub(super) fn append(
    path: &Path,
    document: &Document,
    position: &mut Option<Position>,
) -> Result<(), String> {
    reject_special(path)?;
    if path.exists()
        && fs::metadata(path)
            .map_err(|error| error.to_string())?
            .permissions()
            .readonly()
    {
        return Err("Session file is read-only".into());
    }
    if position.is_none() {
        if path.exists() {
            match read(path, &document.id, &document.directory)? {
                Read::Saved(saved) => {
                    let saved = *saved;
                    if saved.damaged {
                        preserve_tail(path, saved.position.bytes)?;
                    }
                    *position = Some(saved.position);
                }
                Read::NoCheckpoint => {
                    if fs::metadata(path).map_err(|error| error.to_string())?.len() > 0 {
                        preserve_tail(path, 0)?;
                    }
                    *position = Some(Position::default());
                }
            }
        } else {
            *position = Some(Position::default());
        }
    }
    let current = position.as_ref().unwrap();
    if document.messages.len() < current.messages || document.events.len() < current.events {
        return Err("Session history cannot be removed from its journal".into());
    }
    let next_state = state::without_partial(document.state_value());
    let record = Value::object([
        ("format", Value::string("jecode.session.journal")),
        ("format_version", Value::number(3)),
        ("sequence", Value::number(current.sequence)),
        ("messages_from", Value::number(current.messages)),
        ("events_from", Value::number(current.events)),
        (
            "messages",
            Value::Array(document.messages[current.messages..].to_vec()),
        ),
        (
            "events",
            Value::Array(document.events[current.events..].to_vec()),
        ),
        ("state", state::delta(current.state.as_ref(), &next_state)),
        (
            "partial",
            state::partial_delta(&current.partial, &document.pending.partial),
        ),
    ]);
    let bytes = format!("{}\n", record.encode()).into_bytes();
    let mut file = options()
        .read(true)
        .write(true)
        .create_new(current.bytes == 0 && !path.exists())
        .open(path)
        .map_err(|error| format!("Could not open session journal: {error}"))?;
    if file.metadata().map_err(|error| error.to_string())?.len() != current.bytes {
        return Err(
            "Session journal changed outside this session; saved data left unchanged".into(),
        );
    }
    file.seek(SeekFrom::End(0))
        .map_err(|error| error.to_string())?;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        // A failed checkpoint must not become a later, apparently accepted turn.
        // Keep the acknowledged prefix so a staged preparation can be reverted.
        if let Err(rollback) = file.set_len(current.bytes).and_then(|_| file.sync_all()) {
            *position = None;
            return Err(format!(
                "Could not save session checkpoint: {error}; could not restore the preceding checkpoint: {rollback}. Reopen the session before continuing."
            ));
        }
        return Err(format!("Could not save session checkpoint: {error}"));
    }
    let current = position.as_mut().unwrap();
    current.bytes += bytes.len() as u64;
    current.sequence += 1;
    current.messages = document.messages.len();
    current.events = document.events.len();
    current.state = Some(next_state);
    current.partial.clone_from(&document.pending.partial);
    Ok(())
}

fn preserve_tail(path: &Path, length: u64) -> Result<(), String> {
    let mut source = File::open(path).map_err(|error| error.to_string())?;
    source
        .seek(SeekFrom::Start(length))
        .map_err(|error| error.to_string())?;
    let damaged = path.with_extension(format!("jsonl.damaged-{}", identifier()));
    let mut copy = options()
        .write(true)
        .create_new(true)
        .open(damaged)
        .map_err(|error| error.to_string())?;
    std::io::copy(&mut source, &mut copy)
        .and_then(|_| copy.sync_all())
        .map_err(|error| format!("Could not preserve incomplete checkpoint: {error}"))?;
    options()
        .write(true)
        .open(path)
        .and_then(|file| {
            file.set_len(length)?;
            file.sync_all()
        })
        .map_err(|error| format!("Could not repair incomplete checkpoint: {error}"))
}
