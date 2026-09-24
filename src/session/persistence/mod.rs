//! Plain versioned session snapshots. A lease prevents two owners of one conversation.
#[cfg(all(test, any(windows, target_os = "linux")))]
mod access_tests;
mod codec;
#[cfg(all(test, any(windows, target_os = "linux")))]
mod partial_tests;
#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests;
mod transcript;
use super::{Model, history::History, scope::Directory};
use crate::{
    json::{self, Value},
    state::{Lease, Store},
    workspace::{Access, Workspace},
};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
const LIMIT: usize = 16 * 1024 * 1024;

impl History {
    pub(super) fn set_model(&mut self, model: Model) -> io::Result<()> {
        let Some(mut record) = self.record.take() else {
            return Ok(());
        };
        let previous = record.model;
        record.model = model;
        let result = record.save(self);
        if result.is_err() {
            record.model = previous;
        }
        self.record = Some(record);
        result
    }
}

pub struct Saved {
    pub id: String,
    pub model: Model,
    pub workspace: Option<PathBuf>,
    pub directory: Option<PathBuf>,
    pub access: Access,
    pub title: String,
    pub turns: usize,
    pub(super) history: History,
}
pub(super) struct Record {
    store: Store,
    id: String,
    model: Model,
    workspace: Option<String>,
    directory: Option<String>,
    access: Access,
    created: u64,
    extra: BTreeMap<String, Value>,
    _lock: Lease,
}
impl Record {
    pub(super) fn id(&self) -> &str {
        &self.id
    }
    pub(super) fn save(&self, history: &History) -> io::Result<()> {
        let mut fields = vec![
            ("version", Value::Number("1".into())),
            ("id", text(&self.id)),
            ("model", text(self.model.id())),
            ("effort", self.model.effort().map_or(Value::Null, text)),
            (
                "workspace",
                self.workspace.as_deref().map_or(Value::Null, text),
            ),
            ("created", Value::Number(self.created.to_string())),
            ("file_access", text(self.access.name())),
            ("updated", Value::Number(now()?.to_string())),
            ("history", codec::encode(history)),
            (
                "projection",
                json::object([
                    (
                        "through",
                        Value::Number(history.projection.through.to_string()),
                    ),
                    ("step", Value::Number(history.projection.step.to_string())),
                    ("summary", text(&history.projection.summary)),
                    (
                        "limit_bytes",
                        Value::Number(history.projection.limit_bytes.to_string()),
                    ),
                    ("failed", Value::Bool(history.projection.failed)),
                    (
                        "failed_attempts",
                        Value::Array(
                            history
                                .projection
                                .failed_attempts
                                .iter()
                                .map(codec::attempt)
                                .collect(),
                        ),
                    ),
                    ("failed_partial", text(&history.projection.failed_partial)),
                    (
                        "pending",
                        history
                            .projection
                            .pending
                            .as_ref()
                            .map_or(Value::Null, |pending| {
                                json::object([
                                    ("record", Value::Number(pending.record.to_string())),
                                    ("offset", Value::Number(pending.offset.to_string())),
                                    ("summary", text(&pending.summary)),
                                ])
                            }),
                    ),
                ]),
            ),
        ];
        if let Some(directory) = &self.directory {
            fields.push(("directory", text(directory)));
        }
        fields.extend(
            self.extra
                .iter()
                .map(|(key, value)| (key.as_str(), value.clone())),
        );
        let value = json::object(fields);
        let contents = json::encode(&value, LIMIT).map_err(|_| invalid())?;
        self.store.replace(&format!("{}.json", self.id), &contents)
    }
}
#[cfg(test)]
pub(super) fn create(
    store: &Store,
    model: Model,
    workspace: Option<&Workspace>,
) -> io::Result<History> {
    create_in(store, model, workspace.map(Workspace::path), workspace)
}
pub(super) fn create_in(
    store: &Store,
    model: Model,
    directory: Option<&Path>,
    workspace: Option<&Workspace>,
) -> io::Result<History> {
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
        extra: BTreeMap::new(),
        _lock: lock,
        workspace: workspace
            .map(|workspace| {
                workspace
                    .path()
                    .to_str()
                    .map(str::to_owned)
                    .ok_or_else(invalid)
            })
            .transpose()?,
        directory: directory
            .map(|path| path.to_str().map(str::to_owned).ok_or_else(invalid))
            .transpose()?,
        access: workspace.map_or(Access::Workspace, Workspace::access),
    };
    let mut history = History {
        record: Some(record),
        ..Default::default()
    };
    history.projection.limit_bytes = settings.context_limit_bytes;
    history.checkpoint().map_err(|_| invalid())?;
    Ok(history)
}
pub fn resume_in(id: &str, directory: &Directory) -> io::Result<Saved> {
    resume_in_store(&Store::user()?, id, directory)
}
fn resume_in_store(store: &Store, id: &str, directory: &Directory) -> io::Result<Saved> {
    // Diagnose a foreign directory before attempting its single-owner lease.
    let overview = load(store, id, false)?;
    directory.require(overview.directory.as_deref())?;
    drop(overview);
    let saved = load(store, id, true)?;
    // The leased snapshot is authoritative if the file changed since discovery.
    directory.require(saved.directory.as_deref())?;
    Ok(saved)
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
    let effort = match value.get("effort") {
        None => Some("medium"),
        Some(Value::Null) => None,
        Some(Value::String(effort)) => Some(effort.as_str()),
        _ => return Err(invalid()),
    };
    let model = Model::new(string(&value, "model", 128)?, effort).ok_or_else(invalid)?;
    let workspace = match value.get("workspace") {
        Some(Value::Null) => None,
        Some(Value::String(s)) if s.len() <= 32768 && Path::new(s).is_absolute() => {
            Some(PathBuf::from(s))
        }
        _ => return Err(invalid()),
    };
    let directory = match value.get("directory") {
        None => workspace.clone(), // Existing workspace sessions already have an association.
        Some(Value::String(s)) if s.len() <= 32768 && Path::new(s).is_absolute() => {
            Some(PathBuf::from(s))
        }
        _ => return Err(invalid()),
    };
    if workspace.is_some() && directory.is_none() {
        return Err(invalid());
    }
    let created = value
        .get("created")
        .and_then(Value::unsigned)
        .ok_or_else(invalid)?;
    // Missing means the original bounded profile, never today's user default.
    let access = match value.get("file_access") {
        None => Access::Workspace,
        Some(value) => value.text().and_then(Access::parse).ok_or_else(invalid)?,
    };
    if workspace.is_none() && access != Access::Workspace {
        return Err(invalid());
    }
    let mut history = codec::decode(value.get("history").ok_or_else(invalid)?)?;
    let projection = value.get("projection").ok_or_else(invalid)?;
    history.projection.through = projection
        .get("through")
        .and_then(Value::unsigned)
        .filter(|n| {
            *n <= if projection.get("step").is_some() {
                history.turns.len()
            } else {
                history.turns.len().saturating_sub(2)
            } as u64
        })
        .ok_or_else(invalid)? as usize;
    history.projection.step = match projection.get("step") {
        None => 0,
        Some(value) => value
            .unsigned()
            .and_then(|n| n.try_into().ok())
            .ok_or_else(invalid)?,
    };
    if history.projection.step
        > history
            .turns
            .get(history.projection.through)
            .map_or(0, |t| t.steps.len())
    {
        return Err(invalid());
    }
    history.projection.summary = string(projection, "summary", 32768)?.into();
    if (history.projection.through == 0 && history.projection.step == 0)
        != history.projection.summary.is_empty()
    {
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
    history.projection.failed_attempts = match projection.get("failed_attempts") {
        None => Vec::new(),
        Some(Value::Array(items)) if items.len() <= 256 => items
            .iter()
            .map(codec::read_attempt)
            .collect::<io::Result<Vec<_>>>()?,
        _ => return Err(invalid()),
    };
    history.projection.failed_partial = match projection.get("failed_partial") {
        None => String::new(),
        Some(_) => string(projection, "failed_partial", 32768)?.into(),
    };
    history.projection.pending = match projection.get("pending") {
        None | Some(Value::Null) => None,
        Some(value) => Some(super::context::partial::Pending {
            record: value
                .get("record")
                .and_then(Value::unsigned)
                .and_then(|n| n.try_into().ok())
                .ok_or_else(invalid)?,
            offset: value
                .get("offset")
                .and_then(Value::unsigned)
                .and_then(|n| n.try_into().ok())
                .ok_or_else(invalid)?,
            summary: string(value, "summary", 32768)?.into(),
        }),
    };
    if projection.get("step").is_some() && !super::context::valid_cursor(&history) {
        return Err(invalid());
    }
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
            // Do not rewrite older snapshots merely to add an inferred directory.
            directory: value
                .get("directory")
                .and_then(Value::text)
                .map(str::to_owned),
            created,
            extra: match &value {
                Value::Object(fields) => fields
                    .iter()
                    .filter(|(key, _)| {
                        !matches!(
                            key.as_str(),
                            "version"
                                | "id"
                                | "model"
                                | "effort"
                                | "workspace"
                                | "directory"
                                | "created"
                                | "file_access"
                                | "updated"
                                | "history"
                                | "projection"
                        )
                    })
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
                _ => return Err(invalid()),
            },
            access,
            _lock: lock,
        });
    }
    Ok(Saved {
        id: id.into(),
        model,
        workspace,
        directory,
        access,
        title: history
            .turns
            .first()
            .map_or(String::new(), |turn| turn.prompt.clone()),
        turns: history.turns.len(),
        history,
    })
}
pub struct Listed {
    pub id: String,
    pub model: Option<Model>,
    pub title: String,
    pub turns: usize,
    pub workspace: Option<PathBuf>,
    pub directory: Option<PathBuf>,
    pub modified: SystemTime,
}
pub fn list_in(directory: &Directory) -> io::Result<Vec<Listed>> {
    list_in_store(&Store::user()?, directory)
}
fn list_in_store(root: &Store, directory: &Directory) -> io::Result<Vec<Listed>> {
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
    for (modified, name) in names {
        if let Some(id) = name.strip_suffix(".json")
            && valid_id(id)
        {
            let saved = match load(root, id, false) {
                Ok(saved) => Listed {
                    id: saved.id,
                    model: Some(saved.model),
                    title: saved.title,
                    turns: saved.turns,
                    workspace: saved.workspace,
                    directory: saved.directory,
                    modified,
                },
                Err(_) => Listed {
                    id: id.into(),
                    model: None,
                    title: "Unreadable session / file kept on disk".into(),
                    turns: 0,
                    workspace: None,
                    directory: None,
                    modified,
                },
            };
            if saved
                .directory
                .as_deref()
                .is_some_and(|path| directory.contains(path))
            {
                sessions.push(saved);
            }
            if sessions.len() == 50 {
                break;
            }
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
