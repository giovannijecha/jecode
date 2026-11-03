//! Independent MCP connection store. Every mutation holds the same OS lock.
use super::{
    config,
    types::{AuthMethod, Connection, Error, ErrorKind, OAuthTokens},
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

/// The saved record observed just before an interactive authorization.
/// A new grant may replace rotated tokens, but cannot overwrite another edit.
#[derive(Clone)]
pub struct SignInGuard {
    baseline: Box<Connection>,
}

fn same_binding(before: &Option<OAuthTokens>, now: &Option<OAuthTokens>) -> bool {
    match (before, now) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.endpoint == b.endpoint
                && a.issuer == b.issuer
                && a.client_id == b.client_id
                && a.token_endpoint == b.token_endpoint
                && a.resource == b.resource
                && a.github == b.github
                && a.client_auth_method == b.client_auth_method
                && a.client_secret == b.client_secret
        }
        _ => false,
    }
}

fn same_source(before: &Connection, now: &Connection) -> bool {
    let mut expected = before.clone();
    expected.oauth.clone_from(&now.oauth);
    expected.pending_scope.clone_from(&now.pending_scope);
    expected == *now && same_binding(&before.oauth, &now.oauth)
}

fn unconsumed_scope(consumed: Option<&str>, current: Option<&str>) -> Option<String> {
    let consumed = consumed.unwrap_or("").split(' ').collect::<Vec<_>>();
    let remaining = current
        .unwrap_or("")
        .split(' ')
        .filter(|scope| !scope.is_empty() && !consumed.contains(scope))
        .collect::<Vec<_>>();
    (!remaining.is_empty()).then(|| remaining.join(" "))
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
        disk::protect_owner_only(self.path.parent().expect("MCP home"), true)
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
        disk::protect_owner_only(&self.lock, false)
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
        disk::protect_owner_only(&self.path, false)
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
        self.store(connection, false)
    }
    /// Save an edit whose OAuth tokens were loaded with it, not obtained by a
    /// new sign-in. A refresh may have rotated them on disk meanwhile, so the
    /// stored tokens of the same sign-in (endpoint, issuer and client) stay.
    pub fn save_edit(&self, connection: &Connection) -> Result<Connection, Error> {
        self.store(connection, true)
    }
    /// Refresh a saved draft's source before opening the browser. Deliberate
    /// edits on the draft are retained; a new endpoint has no old scope.
    pub fn prepare_sign_in(
        &self,
        draft: &Connection,
        original: Option<&Connection>,
    ) -> Result<(Connection, Option<SignInGuard>), Error> {
        let Some(original) = original else {
            return Ok((draft.clone(), None));
        };
        let _guard = self.lock()?;
        let document = self.document()?;
        let stored = Self::connections(&document)?
            .iter()
            .find(|entry| entry.get("id").and_then(Value::as_str) == Some(original.id.as_str()))
            .ok_or_else(|| failure("MCP connection was removed; reload it before sign-in"))
            .and_then(config::read)?;
        if !same_source(original, &stored) {
            return Err(failure("MCP connection changed; reload it before sign-in"));
        }
        let mut prepared = draft.clone();
        if prepared.endpoint == stored.endpoint {
            if prepared.method == AuthMethod::Browser {
                prepared.pending_scope.clone_from(&stored.pending_scope);
            }
            if prepared.oauth == original.oauth {
                prepared.oauth.clone_from(&stored.oauth);
            }
        }
        Ok((
            prepared,
            Some(SignInGuard {
                baseline: Box::new(stored),
            }),
        ))
    }

    /// Commit a new grant only if its saved source still exists and has not
    /// changed. A challenge arriving during browser sign-in remains pending.
    pub fn commit_sign_in(
        &self,
        connection: &Connection,
        guard: &SignInGuard,
    ) -> Result<Connection, Error> {
        config::validate(connection)?;
        if connection.id != guard.baseline.id {
            return Err(failure("MCP sign-in connection changed; reload it"));
        }
        let _lock = self.lock()?;
        let mut document = self.document()?;
        let entries = Self::entries(&mut document)?;
        if entries.iter().any(|entry| {
            entry.get("id").and_then(Value::as_str) != Some(connection.id.as_str())
                && entry
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name.eq_ignore_ascii_case(&connection.name))
        }) {
            return Err(failure("MCP connection name already exists"));
        }
        let entry = entries
            .iter_mut()
            .find(|entry| {
                entry.get("id").and_then(Value::as_str) == Some(guard.baseline.id.as_str())
            })
            .ok_or_else(|| failure("MCP connection was removed during sign-in"))?;
        let current = config::read(entry)?;
        if !same_source(&guard.baseline, &current) {
            return Err(failure("MCP connection changed during sign-in; reload it"));
        }
        let mut next = connection.clone();
        // The authorization consumed the scopes present at preparation. A
        // later challenge may still need another explicit authorization.
        next.pending_scope = (next.endpoint == guard.baseline.endpoint)
            .then(|| {
                unconsumed_scope(
                    guard.baseline.pending_scope.as_deref(),
                    current.pending_scope.as_deref(),
                )
            })
            .flatten();
        config::validate(&next)?;
        *entry = config::write(&next, Some(entry.clone()));
        self.write(&document)?;
        Ok(next)
    }
    fn store(&self, connection: &Connection, keep_tokens: bool) -> Result<Connection, Error> {
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
            if keep_tokens && let Some(ours) = &next.oauth {
                let stored = existing[index]
                    .oauth
                    .as_ref()
                    .ok_or_else(|| failure("MCP OAuth session changed; reload the connection"))?;
                if ours.endpoint != stored.endpoint
                    || ours.issuer != stored.issuer
                    || ours.client_id != stored.client_id
                    || ours.token_endpoint != stored.token_endpoint
                    || ours.resource != stored.resource
                    || ours.github != stored.github
                {
                    return Err(failure("MCP OAuth session changed; reload the connection"));
                }
                next.oauth = Some(stored.clone());
                // An edit opened before a step-up challenge must retain the
                // scope recorded while the form was open.
                next.pending_scope
                    .clone_from(&existing[index].pending_scope);
            }
            entries[index] = config::write(&next, Some(entries[index].clone()));
        } else if keep_tokens && !next.id.is_empty() {
            return Err(failure("MCP connection was removed; reload the connection"));
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
    /// Remember a tool-specific OAuth challenge only for the exact session
    /// that received it. A late response cannot affect a new sign-in.
    pub fn require_scope(&self, current: &Connection, scope: &str) -> Result<bool, Error> {
        if current.method != super::types::AuthMethod::Browser {
            return Err(failure("MCP scope step-up requires browser OAuth"));
        }
        let _guard = self.lock()?;
        let mut document = self.document()?;
        let entries = Self::entries(&mut document)?;
        let Some(entry) = entries
            .iter_mut()
            .find(|entry| entry.get("id").and_then(Value::as_str) == Some(current.id.as_str()))
        else {
            return Ok(false);
        };
        let stored = config::read(entry)?;
        let mut expected = current.clone();
        expected.pending_scope.clone_from(&stored.pending_scope);
        if stored != expected {
            return Ok(false);
        }
        let merged = super::auth::scopes::union(&[stored.pending_scope.as_deref(), Some(scope)])?;
        if merged != stored.pending_scope {
            let mut updated = stored;
            updated.pending_scope = merged;
            config::validate(&updated)?;
            let prior = entry.clone();
            *entry = config::write(&updated, Some(prior));
            self.write(&document)?;
        }
        Ok(true)
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
mod stepup_tests;
#[cfg(test)]
mod tests;
