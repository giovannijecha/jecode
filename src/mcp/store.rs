//! Independent MCP connection store. Every mutation holds the same OS lock.
use super::{
    config,
    types::{Connection, Error, ErrorKind},
};
use crate::{
    cancel::Cancellation,
    disk,
    json::{self, Value},
};
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::path::Path;
use std::{
    fs::File,
    io::Read,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const LIMIT: usize = 256 * 1024;
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn failure(message: &str) -> Error {
    Error::new(ErrorKind::Configuration, message)
}
fn io(_: std::io::Error) -> Error {
    failure("Could not access MCP configuration")
}
fn cancelled() -> Error {
    Error::new(ErrorKind::Cancelled, "Operation cancelled")
}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
    lock: PathBuf,
}

impl Store {
    pub fn new(home: PathBuf) -> Self {
        Self {
            path: home.join("mcp.json"),
            lock: home.join("mcp.lock"),
        }
    }
    #[cfg(test)]
    fn path(&self) -> &Path {
        &self.path
    }

    fn document(&self) -> Result<Value, Error> {
        let Some(meta) = disk::metadata_if_present(&self.path).map_err(io)? else {
            return Ok(Value::object([]));
        };
        if !meta.is_file() || disk::linked(&meta) || meta.len() > LIMIT as u64 {
            return Err(failure(
                "MCP configuration must be a regular file under 256 KiB",
            ));
        }
        let mut bytes = Vec::new();
        File::open(&self.path)
            .map_err(io)?
            .take(LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(io)?;
        if bytes.len() > LIMIT {
            return Err(failure("MCP configuration exceeds 256 KiB"));
        }
        let text =
            std::str::from_utf8(&bytes).map_err(|_| failure("MCP configuration is not UTF-8"))?;
        let document =
            json::parse(text).map_err(|_| failure("MCP configuration is invalid JSON"))?;
        if !matches!(document, Value::Object(_)) {
            return Err(failure("MCP configuration must be an object"));
        }
        Self::connections(&document)?;
        Ok(document)
    }
    fn connections(document: &Value) -> Result<&[Value], Error> {
        match document.get("connections") {
            None => Ok(&[]),
            Some(Value::Array(values)) => Ok(values),
            _ => Err(failure("MCP connections must be an array")),
        }
    }
    fn entries(document: &mut Value) -> Result<&mut Vec<Value>, Error> {
        let Value::Object(map) = document else {
            unreachable!()
        };
        let value = map
            .entry("connections".into())
            .or_insert_with(|| Value::Array(Vec::new()));
        let Value::Array(values) = value else {
            return Err(failure("MCP connections must be an array"));
        };
        Ok(values)
    }
    fn lock(&self) -> Result<disk::lock::Exclusive, Error> {
        self.acquire(None)
    }
    /// Waiters stay cancellable while another caller refreshes under the lock.
    fn acquire(&self, cancel: Option<&Cancellation>) -> Result<disk::lock::Exclusive, Error> {
        disk::ensure_private_directory(self.path.parent().expect("MCP home")).map_err(io)?;
        #[cfg(windows)]
        crate::auth::openai::native::protect(self.path.parent().expect("MCP home"), true)
            .map_err(|_| failure("Could not protect MCP directory"))?;
        if disk::metadata_if_present(&self.lock)
            .map_err(io)?
            .as_ref()
            .is_some_and(|m| !m.is_file() || disk::linked(m))
        {
            return Err(failure("MCP lock path is invalid"));
        }
        let lock = match cancel {
            Some(cancel) => disk::lock::Exclusive::acquire_cancellable(&self.lock, cancel)
                .map_err(|e| {
                    if cancel.requested() {
                        cancelled()
                    } else {
                        io(e)
                    }
                })?,
            None => disk::lock::Exclusive::acquire(&self.lock).map_err(io)?,
        };
        #[cfg(windows)]
        crate::auth::openai::native::protect(&self.lock, false)
            .map_err(|_| failure("Could not protect MCP lock file"))?;
        Ok(lock)
    }
    fn write(&self, document: &Value) -> Result<(), Error> {
        let bytes = format!("{}\n", document.pretty());
        if bytes.len() > LIMIT {
            return Err(failure("MCP configuration exceeds 256 KiB"));
        }
        if disk::metadata_if_present(&self.path)
            .map_err(io)?
            .as_ref()
            .is_some_and(|m| !m.is_file() || disk::linked(m) || m.permissions().readonly())
        {
            return Err(failure("MCP configuration path is invalid or read-only"));
        }
        disk::atomic_write(&self.path, bytes.as_bytes()).map_err(io)?;
        #[cfg(windows)]
        crate::auth::openai::native::protect(&self.path, false)
            .map_err(|_| failure("Could not protect MCP configuration"))?;
        Ok(())
    }
    pub fn list(&self) -> Result<Vec<Connection>, Error> {
        let document = self.document()?;
        let connections = Self::connections(&document)?
            .iter()
            .map(config::read)
            .collect::<Result<Vec<_>, _>>()?;
        Self::unique(&connections)?;
        Ok(connections)
    }
    fn unique(connections: &[Connection]) -> Result<(), Error> {
        for (i, entry) in connections.iter().enumerate() {
            if connections[..i]
                .iter()
                .any(|other| other.id == entry.id || other.name.eq_ignore_ascii_case(&entry.name))
            {
                return Err(failure("MCP connection IDs and names must be unique"));
            }
        }
        Ok(())
    }
    pub fn save(&self, connection: &Connection) -> Result<Connection, Error> {
        config::validate(connection)?;
        let _guard = self.lock()?;
        let mut document = self.document()?;
        let entries = Self::entries(&mut document)?;
        let existing = entries
            .iter()
            .map(config::read)
            .collect::<Result<Vec<_>, _>>()?;
        Self::unique(&existing)?;
        let mut next = connection.clone();
        if next.id.is_empty() {
            next.id = make_id(&existing);
        }
        if existing
            .iter()
            .any(|entry| entry.id != next.id && entry.name.eq_ignore_ascii_case(&next.name))
        {
            return Err(failure("MCP connection name already exists"));
        }
        if let Some(index) = existing.iter().position(|entry| entry.id == next.id) {
            entries[index] = config::write(&next, Some(entries[index].clone()));
        } else {
            entries.push(config::write(&next, None));
        }
        self.write(&document)?;
        Ok(next)
    }
    pub fn remove(&self, id: &str) -> Result<(), Error> {
        let _guard = self.lock()?;
        let mut document = self.document()?;
        let entries = Self::entries(&mut document)?;
        let before = entries.len();
        entries.retain(|value| value.get("id").and_then(Value::as_str) != Some(id));
        if entries.len() == before {
            return Err(failure("MCP connection was not found"));
        }
        self.write(&document)
    }
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), Error> {
        let _guard = self.lock()?;
        let mut document = self.document()?;
        let entries = Self::entries(&mut document)?;
        let entry = entries
            .iter_mut()
            .find(|value| value.get("id").and_then(Value::as_str) == Some(id))
            .ok_or_else(|| failure("MCP connection was not found"))?;
        let Value::Object(map) = entry else {
            return Err(failure("MCP connection is invalid"));
        };
        map.insert("enabled".into(), Value::Bool(enabled));
        self.write(&document)
    }
    /// Read and renew only the current record while holding the mutation lock.
    pub fn fresh(&self, id: &str, cancel: &Cancellation) -> Result<Connection, Error> {
        if cancel.requested() {
            return Err(cancelled());
        }
        let _guard = self.acquire(Some(cancel))?;
        let mut document = self.document()?;
        let entries = Self::entries(&mut document)?;
        let index = entries
            .iter()
            .position(|value| value.get("id").and_then(Value::as_str) == Some(id))
            .ok_or_else(|| failure("MCP connection was removed"))?;
        let current = config::read(&entries[index])?;
        if !current.enabled {
            return Err(failure("MCP connection is disabled"));
        }
        let expires = current.oauth.as_ref().and_then(|oauth| oauth.expires_at);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| failure("System clock is invalid"))?
            .as_secs();
        if expires.is_none_or(|at| at > now.saturating_add(60)) {
            return Ok(current);
        }
        let refreshed = super::auth::refresh(&current, cancel)?;
        entries[index] = config::write(&refreshed, Some(entries[index].clone()));
        self.write(&document)?;
        Ok(refreshed)
    }
}
fn make_id(existing: &[Connection]) -> String {
    loop {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let count = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let id = format!("mcp-{time:x}-{count:x}");
        if !existing.iter().any(|entry| entry.id == id) {
            return id;
        }
    }
}

#[cfg(test)]
mod tests;
