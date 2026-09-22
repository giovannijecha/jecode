//! Plain versioned session snapshots. A lease prevents two owners of one conversation.
mod codec;
#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests;
mod transcript;
use super::{Model, history::History};
use crate::{
    json::{self, Value},
    state::Store,
};
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
const LIMIT: usize = 16 * 1024 * 1024;

pub struct Saved {
    pub id: String,
    pub model: Model,
    pub workspace: Option<PathBuf>,
    pub title: String,
    pub turns: usize,
    pub(super) history: History,
}
pub(super) struct Record {
    store: Store,
    id: String,
    model: Model,
    workspace: Option<String>,
    created: u64,
    _lock: File,
}
impl Record {
    pub(super) fn id(&self) -> &str {
        &self.id
    }
    pub(super) fn save(&self, history: &History) -> io::Result<()> {
        let value = json::object([
            ("version", Value::Number("1".into())),
            ("id", text(&self.id)),
            ("model", text(self.model.id())),
            (
                "workspace",
                self.workspace.as_deref().map_or(Value::Null, text),
            ),
            ("created", Value::Number(self.created.to_string())),
            ("updated", Value::Number(now()?.to_string())),
            ("history", codec::encode(history)),
            (
                "projection",
                json::object([
                    (
                        "through",
                        Value::Number(history.projection.through.to_string()),
                    ),
                    ("summary", text(&history.projection.summary)),
                    (
                        "limit_bytes",
                        Value::Number(history.projection.limit_bytes.to_string()),
                    ),
                    ("failed", Value::Bool(history.projection.failed)),
                ]),
            ),
        ]);
        let contents = json::encode(&value, LIMIT).map_err(|_| invalid())?;
        self.store.replace(&format!("{}.json", self.id), &contents)
    }
}
pub(super) fn create(store: &Store, model: Model, workspace: Option<&Path>) -> io::Result<History> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let settings = crate::state::settings::Settings::load(store)?;
    let store = store.directory("sessions")?;
    let created = now()?;
    let id = format!(
        "s-{created:016x}-{:08x}-{:08x}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let lock = store.lock(
        &format!("{id}.lock"),
        &AtomicBool::new(false),
        Instant::now(),
    )?;
    if store.read(&format!("{id}.json"), LIMIT)?.is_some() {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    let record = Record {
        store,
        id,
        model,
        created,
        _lock: lock,
        workspace: workspace
            .map(|path| path.to_str().map(str::to_owned).ok_or_else(invalid))
            .transpose()?,
    };
    let mut history = History {
        record: Some(record),
        ..Default::default()
    };
    history.projection.limit_bytes = settings.context_limit_bytes;
    history.checkpoint().map_err(|_| invalid())?;
    Ok(history)
}
pub fn resume(id: &str) -> io::Result<Saved> {
    load(&Store::user()?, id, true)
}

fn load(store: &Store, id: &str, leased: bool) -> io::Result<Saved> {
    if !valid_id(id) {
        return Err(invalid());
    }
    let store = store.directory("sessions")?;
    let lock = if leased {
        Some(store.lock(
            &format!("{id}.lock"),
            &AtomicBool::new(false),
            Instant::now(),
        )?)
    } else {
        None
    };
    let contents = store
        .read(&format!("{id}.json"), LIMIT)?
        .ok_or(io::ErrorKind::NotFound)?;
    let value = json::parse(
        &contents,
        json::Limits {
            bytes: LIMIT,
            nodes: 500_000,
            depth: 64,
        },
    )
    .map_err(|_| invalid())?;
    if value.get("version").and_then(Value::unsigned) != Some(1) || string(&value, "id", 64)? != id
    {
        return Err(invalid());
    }
    let model = match string(&value, "model", 64)? {
        "gpt-5.6-luna" => Model::Luna,
        "gpt-5.6-terra" => Model::Terra,
        _ => return Err(invalid()),
    };
    let workspace = match value.get("workspace") {
        Some(Value::Null) => None,
        Some(Value::String(s)) if s.len() <= 32768 && Path::new(s).is_absolute() => {
            Some(PathBuf::from(s))
        }
        _ => return Err(invalid()),
    };
    let created = value
        .get("created")
        .and_then(Value::unsigned)
        .ok_or_else(invalid)?;
    let mut history = codec::decode(value.get("history").ok_or_else(invalid)?)?;
    let projection = value.get("projection").ok_or_else(invalid)?;
    history.projection.through = projection
        .get("through")
        .and_then(Value::unsigned)
        .filter(|n| *n <= history.turns.len().saturating_sub(2) as u64)
        .ok_or_else(invalid)? as usize;
    history.projection.summary = string(projection, "summary", 32768)?.into();
    if (history.projection.through == 0) != history.projection.summary.is_empty() {
        return Err(invalid());
    }
    history.projection.limit_bytes = projection
        .get("limit_bytes")
        .and_then(Value::unsigned)
        .filter(|n| (65536..=1572864).contains(n))
        .ok_or_else(invalid)? as usize;
    history.projection.failed = match projection.get("failed") {
        Some(Value::Bool(failed)) => *failed,
        _ => return Err(invalid()),
    };
    if let Some(lock) = lock {
        if let Some(turn) = history.turns.last_mut()
            && turn.end.is_none()
        {
            turn.end = Some(super::End::Failed(super::Failure::Worker));
            turn.outcome = "Interrupted session / no operation was replayed. Verify any unknown tool outcome before continuing.".into();
        }
        history.record = Some(Record {
            store,
            id: id.into(),
            model,
            workspace: workspace.as_ref().map(|p| p.to_string_lossy().into_owned()),
            created,
            _lock: lock,
        });
    }
    Ok(Saved {
        id: id.into(),
        model,
        workspace,
        title: history.turns.first().map_or(String::new(), |turn| {
            turn.prompt.chars().take(100).collect()
        }),
        turns: history.turns.len(),
        history,
    })
}
pub struct Listed {
    pub id: String,
    pub model: Option<Model>,
    pub title: String,
    pub turns: usize,
}
pub fn list() -> io::Result<Vec<Listed>> {
    let root = Store::user()?;
    let store = root.directory("sessions")?;
    let mut names: Vec<_> = store
        .names()?
        .into_iter()
        .filter(|name| name.strip_suffix(".json").is_some_and(valid_id))
        .map(|name| {
            let modified = std::fs::symlink_metadata(store.root().join(&name))
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
            (modified, name)
        })
        .collect();
    names.sort_by(|a, b| b.cmp(a));
    let mut sessions = Vec::new();
    for (_, name) in names.into_iter().take(50) {
        if let Some(id) = name.strip_suffix(".json")
            && valid_id(id)
        {
            sessions.push(match load(&root, id, false) {
                Ok(saved) => Listed {
                    id: saved.id,
                    model: Some(saved.model),
                    title: saved.title,
                    turns: saved.turns,
                },
                Err(_) => Listed {
                    id: id.into(),
                    model: None,
                    title: "Unreadable session / file kept on disk".into(),
                    turns: 0,
                },
            });
        }
    }
    Ok(sessions)
}
fn valid_id(id: &str) -> bool {
    id.starts_with("s-")
        && id.len() == 36
        && id[2..].bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}
fn now() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|n| n.as_secs())
        .map_err(|_| invalid())
}
fn invalid() -> io::Error {
    io::Error::other("invalid or unsupported saved session")
}
fn text(s: &str) -> Value {
    Value::String(s.into())
}
fn string<'a>(value: &'a Value, key: &str, max: usize) -> io::Result<&'a str> {
    value
        .get(key)
        .and_then(Value::text)
        .filter(|s| s.len() <= max)
        .ok_or_else(invalid)
}
