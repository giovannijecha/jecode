use super::{Record, Tracker, invalid, text};
use crate::{
    json::{self, Value},
    session::{Model, context::partial::Pending, history::History},
    state::Store,
    workspace::Access,
};
use std::{
    io,
    path::{Path, PathBuf},
};

pub(super) const HEAD_LIMIT: usize = 1024 * 1024;

pub(super) struct Info {
    pub model: Model,
    pub workspace: Option<PathBuf>,
    pub directory: Option<PathBuf>,
    pub access: Access,
    pub created: u64,
    pub committed: u64,
    pub rolling: u64,
    pub turns: usize,
    pub through: usize,
    pub step: usize,
    pub guidance_base: usize,
    pub verified: bool,
    pub title: String,
    pub recent: Vec<String>,
    pub(super) projection: Value,
}

fn number(n: impl ToString) -> Value {
    Value::Number(n.to_string())
}
fn field<'a>(value: &'a Value, key: &str, max: usize) -> io::Result<&'a str> {
    value
        .get(key)
        .and_then(Value::text)
        .filter(|s| s.len() <= max)
        .ok_or_else(invalid)
}
fn integer(value: &Value, key: &str) -> io::Result<usize> {
    value
        .get(key)
        .and_then(Value::unsigned)
        .and_then(|n| n.try_into().ok())
        .ok_or_else(invalid)
}
fn path(value: &Value, key: &str) -> io::Result<Option<PathBuf>> {
    match value.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(path)) if path.len() <= 32768 && Path::new(path).is_absolute() => {
            Ok(Some(PathBuf::from(path)))
        }
        _ => Err(invalid()),
    }
}

pub(super) fn projection(history: &History) -> Value {
    let p = &history.projection;
    let (through, step, guidance_base) = history.projected_cursor();
    json::object([
        ("through", number(through)),
        ("step", number(step)),
        ("guidance_base", number(guidance_base)),
        ("summary", text(&p.summary)),
        ("limit_bytes", number(p.limit_bytes)),
        ("failed", Value::Bool(p.failed)),
        (
            "failed_at_turn",
            p.failed_at_turn.map_or(Value::Null, number),
        ),
        (
            "failed_attempts",
            Value::Array(
                p.failed_attempts
                    .iter()
                    .map(super::super::codec::attempt)
                    .collect(),
            ),
        ),
        ("failed_partial", text(&p.failed_partial)),
        (
            "pending",
            p.pending.as_ref().map_or(Value::Null, |pending| {
                json::object([
                    ("record", number(pending.record)),
                    ("offset", number(pending.offset)),
                    ("summary", text(&pending.summary)),
                ])
            }),
        ),
    ])
}

pub(super) fn write(record: &Record, history: &History, tracker: &mut Tracker) -> io::Result<()> {
    let mut head = json::object([
        ("version", number(2)),
        ("verified", Value::Bool(tracker.verified)),
        ("id", text(&record.id)),
        ("model", text(record.model.id())),
        ("effort", record.model.effort().map_or(Value::Null, text)),
        (
            "workspace",
            record.workspace.as_deref().map_or(Value::Null, text),
        ),
        (
            "directory",
            record.directory.as_deref().map_or(Value::Null, text),
        ),
        ("file_access", text(record.access.name())),
        ("created", number(record.created)),
        ("updated", number(super::super::now()?)),
        ("committed", number(tracker.committed)),
        ("rolling", number(tracker.rolling)),
        ("turns", number(tracker.turns)),
        ("title", text(&tracker.title)),
        (
            "recent",
            Value::Array(tracker.recent.iter().map(|s| text(s)).collect()),
        ),
        ("projection", projection(history)),
    ]);
    loop {
        // The actual encoded head includes the title, projection, paths and
        // integrity field. A count bound alone cannot account for JSON escaping.
        let integrity = super::log::fingerprint(&head)?;
        let mut candidate = head.clone();
        if let Value::Object(fields) = &mut candidate {
            fields.insert("integrity".into(), number(integrity));
        }
        match json::encode(&candidate, HEAD_LIMIT) {
            Ok(contents) => {
                return record
                    .store
                    .replace(&format!("{}.head", record.id), &contents);
            }
            Err(json::Error::Limit) if !tracker.recent.is_empty() => {
                tracker.recent.remove(0);
                let Value::Object(fields) = &mut head else {
                    return Err(invalid());
                };
                let Some(Value::Array(recent)) = fields.get_mut("recent") else {
                    return Err(invalid());
                };
                recent.remove(0);
            }
            Err(_) => return Err(invalid()),
        }
    }
}

pub(super) fn read(store: &Store, id: &str) -> io::Result<Info> {
    let source = store
        .read(&format!("{id}.head"), HEAD_LIMIT)?
        .ok_or(io::ErrorKind::NotFound)?;
    let mut value = json::parse(
        &source,
        json::Limits {
            bytes: HEAD_LIMIT,
            nodes: 100_000,
            depth: 32,
        },
    )
    .map_err(|_| corrupt_head())?;
    let integrity = value
        .get("integrity")
        .and_then(Value::unsigned)
        .ok_or_else(corrupt_head)?;
    if let Value::Object(fields) = &mut value {
        fields.remove("integrity");
    }
    if super::log::fingerprint(&value)? != integrity {
        return Err(corrupt_head());
    }
    if value.get("version").and_then(Value::unsigned) != Some(2) || field(&value, "id", 64)? != id {
        return Err(invalid());
    }
    let effort = match value.get("effort") {
        Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.as_str()),
        _ => return Err(invalid()),
    };
    let model = Model::new(field(&value, "model", 128)?, effort).ok_or_else(invalid)?;
    let workspace = path(&value, "workspace")?;
    let directory = path(&value, "directory")?;
    if workspace.is_some() && directory.is_none() {
        return Err(invalid());
    }
    let access = Access::parse(field(&value, "file_access", 32)?).ok_or_else(invalid)?;
    if workspace.is_none() && access != Access::Workspace {
        return Err(invalid());
    }
    let turns = integer(&value, "turns")?;
    let verified = match value.get("verified") {
        Some(Value::Bool(value)) => *value,
        _ => return Err(invalid()),
    };
    let projection = value.get("projection").cloned().ok_or_else(invalid)?;
    let through = integer(&projection, "through")?;
    let step = integer(&projection, "step")?;
    let guidance_base = integer(&projection, "guidance_base")?;
    if through > turns {
        return Err(invalid());
    }
    let recent = value
        .get("recent")
        .and_then(Value::array)
        .filter(|items| items.len() <= super::super::super::MAX_RECALLED_PROMPTS)
        .ok_or_else(invalid)?
        .iter()
        .map(|item| {
            item.text()
                .filter(|s| s.len() <= super::super::super::MAX_PROMPT_BYTES)
                .map(str::to_owned)
                .ok_or_else(invalid)
        })
        .collect::<io::Result<Vec<_>>>()?;
    Ok(Info {
        model,
        workspace,
        directory,
        access,
        created: value
            .get("created")
            .and_then(Value::unsigned)
            .ok_or_else(invalid)?,
        committed: value
            .get("committed")
            .and_then(Value::unsigned)
            .ok_or_else(invalid)?,
        rolling: value
            .get("rolling")
            .and_then(Value::unsigned)
            .ok_or_else(invalid)?,
        turns,
        through,
        step,
        guidance_base,
        verified,
        title: field(&value, "title", super::super::super::MAX_PROMPT_BYTES)?.into(),
        recent,
        projection,
    })
}

fn corrupt_head() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "committed session head is corrupt",
    )
}

pub(super) fn restore_projection(history: &mut History, info: &Info) -> io::Result<()> {
    let value = &info.projection;
    if integer(value, "through")? != info.through {
        return Err(invalid());
    }
    let step = integer(value, "step")?;
    if step != info.step || step > 0 && history.turns.is_empty() {
        return Err(invalid());
    }
    let summary = field(value, "summary", 32768)?;
    if (info.through == 0 && step == 0) != summary.is_empty() {
        return Err(invalid());
    }
    let limit = integer(value, "limit_bytes")?;
    if !(65536..=1572864).contains(&limit) {
        return Err(invalid());
    }
    history.projection.through = 0;
    history.projection.step = 0;
    history.projection.summary = summary.into();
    history.projection.limit_bytes = limit;
    history.projection.failed = match value.get("failed") {
        Some(Value::Bool(v)) => *v,
        _ => return Err(invalid()),
    };
    history.projection.failed_at_turn = match value.get("failed_at_turn") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .unsigned()
                .and_then(|turn| turn.try_into().ok())
                .filter(|turn| *turn <= history.turn_count())
                .ok_or_else(invalid)?,
        ),
    };
    history.projection.failed_attempts = match value.get("failed_attempts") {
        Some(Value::Array(items)) if items.len() <= 256 => items
            .iter()
            .map(super::super::codec::read_attempt)
            .collect::<io::Result<Vec<_>>>()?,
        _ => return Err(invalid()),
    };
    history.projection.failed_partial = field(value, "failed_partial", 32768)?.into();
    history.projection.pending = match value.get("pending") {
        Some(Value::Null) => None,
        Some(pending) => Some(Pending {
            record: integer(pending, "record")?,
            offset: integer(pending, "offset")?,
            summary: field(pending, "summary", 32768)?.into(),
        }),
        None => return Err(invalid()),
    };
    Ok(())
}
