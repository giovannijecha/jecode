mod check;
pub use check::check;
mod control;
mod diagnostics;
mod error;
mod idle;
mod outcome;
mod root;
mod session;
#[cfg(test)]
mod tests;

use diagnostics::diagnostics;
use error::QueryFailure;
use root::choose_root;

use super::Profile;
use super::client::Client;
use super::locations;
use crate::cancel::Cancellation;
use crate::json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub enum Request {
    Status {
        server: Option<String>,
    },
    Definition {
        path: PathBuf,
        line: u32,
        column: u32,
        server: Option<String>,
    },
    References {
        path: PathBuf,
        line: u32,
        column: u32,
        include_declaration: bool,
        server: Option<String>,
    },
    Diagnostics {
        path: PathBuf,
        server: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub struct Status {
    pub server: String,
    pub root: Option<PathBuf>,
    pub state: String,
    pub detail: String,
    pub capabilities: Option<Value>,
    pub pid: Option<u32>,
    pub generation: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub status: String,
    pub server: Option<String>,
    pub root: Option<PathBuf>,
    pub locations: Vec<Value>,
    pub diagnostics: Vec<Value>,
    pub freshness: String,
    pub complete: bool,
    pub truncated: bool,
    pub detail: String,
    pub servers: Vec<Status>,
    pub document_version: Option<i64>,
    pub instance_generation: Option<u64>,
}

struct Entry {
    id: String,
    root: PathBuf,
    root_source: String,
    fingerprint: String,
    stop: Cancellation,
    dirty: AtomicU64,
    /// Requests queued or running; only these make the instance `busy`.
    requests: AtomicUsize,
    /// What `statuses` reports, published whenever a session releases `state`.
    observed: Mutex<Status>,
    state: Mutex<State>,
}
struct State {
    client: Option<Client>,
    generation: u64,
    failures: u8,
    retry_at: Option<Instant>,
    status: String,
    detail: String,
}
impl State {
    /// Records a crash; consecutive crashes back off and, from the third, suppress restarts.
    fn crashed(&mut self, error: &str) {
        self.client.take();
        self.failures = self.failures.saturating_add(1);
        self.retry_at = Some(Instant::now() + Duration::from_secs(1 << self.failures.min(3)));
        self.status = if self.failures >= 3 {
            "restart_suppressed"
        } else {
            "crashed"
        }
        .into();
        self.detail = error.into();
    }
}
struct Inner {
    home: Option<PathBuf>,
    entries: Mutex<HashMap<(String, PathBuf), Arc<Entry>>>,
    redactor: Mutex<Option<crate::redact::Redactor>>,
    retiring: Mutex<Vec<std::thread::JoinHandle<()>>>,
}
#[derive(Clone)]
pub struct Runtime(Arc<Inner>);

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}
impl Runtime {
    pub fn new() -> Self {
        Self::build(None)
    }
    pub fn with_home(home: PathBuf) -> Self {
        Self::build(Some(home))
    }
    fn build(home: Option<PathBuf>) -> Self {
        Self(Arc::new(Inner {
            home,
            entries: Mutex::new(HashMap::new()),
            redactor: Mutex::new(None),
            retiring: Mutex::new(Vec::new()),
        }))
    }

    pub fn execute(
        &self,
        workspace: &Path,
        profiles: &[Profile],
        request: Request,
        cancel: &Cancellation,
    ) -> Outcome {
        let (path, server) = match &request {
            Request::Status { server } => {
                return self.status(workspace, profiles, server.as_deref());
            }
            Request::Definition { path, server, .. }
            | Request::References { path, server, .. }
            | Request::Diagnostics { path, server } => (path, server.as_deref()),
        };
        let profiles = match self.authoritative(profiles) {
            Ok(v) => v,
            Err(e) => return Outcome::basic("configuration_error", e),
        };
        self.prune(&profiles);
        let path = if path.is_absolute() {
            path.clone()
        } else {
            workspace.join(path)
        };
        let path = match std::fs::canonicalize(&path) {
            Ok(v) => v,
            Err(e) => return Outcome::basic("unavailable", format!("LSP path unavailable: {e}")),
        };
        let mut matches = profiles
            .iter()
            .filter(|p| p.matches(&path) && server.is_none_or(|id| id == p.id))
            .collect::<Vec<_>>();
        if matches.is_empty() {
            return Outcome::basic("unavailable", "No configured LSP server matches this path");
        }
        if server.is_none() {
            let enabled = matches
                .iter()
                .copied()
                .filter(|profile| profile.enabled)
                .collect::<Vec<_>>();
            if enabled.is_empty() {
                return Outcome::basic("off", "Matching LSP servers are disabled");
            }
            matches = enabled;
        }
        if matches.len() > 1 && server.is_none() {
            return Outcome::basic("ambiguous", "Multiple LSP servers match; specify server ID");
        }
        let profile = matches.remove(0);
        if !profile.enabled {
            return Outcome::basic("off", "LSP server is disabled");
        }
        let (root, root_source) = match choose_root(workspace, &path, profile) {
            Ok(value) => value,
            Err(error) => return Outcome::basic("configuration_error", error),
        };
        let entry = self.entry(profile, &root, &root_source);
        // Declared before the session so it stops counting after the release.
        let _pending = entry.pending();
        let dirty_before = entry.dirty.load(Ordering::Acquire);
        let mut outcome = Outcome::basic("ready", "");
        outcome.server = Some(profile.id.clone());
        outcome.root = Some(root.clone());
        let queue_deadline = Instant::now() + Duration::from_millis(profile.request_timeout_ms);
        let mut state = loop {
            if cancel.requested() {
                return Outcome::basic("cancelled", "LSP request cancelled before dispatch");
            }
            if entry.stop.requested() {
                return Outcome::basic("settings_changed", "LSP settings changed before dispatch");
            }
            match entry.try_session() {
                Ok(state) => break state,
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    return Outcome::basic("unavailable", "LSP state unavailable");
                }
                Err(std::sync::TryLockError::WouldBlock) => {
                    if Instant::now() >= queue_deadline {
                        return Outcome::basic(
                            "timed_out",
                            "LSP session remained busy before dispatch",
                        );
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        };
        if cancel.requested() {
            return Outcome::basic("cancelled", "LSP request cancelled before dispatch");
        }
        if entry.stop.requested() {
            return Outcome::basic("settings_changed", "LSP settings changed");
        }
        if state.failures >= 3 {
            state.status = "restart_suppressed".into();
            return self.fail(&mut outcome, "restart_suppressed", &state.detail);
        }
        if state.client.is_none() {
            if state.retry_at.is_some_and(|at| Instant::now() < at) {
                return self.fail(&mut outcome, "crashed", &state.detail);
            }
            state.status = "starting".into();
            state.generation += 1;
            let generation = state.generation;
            let started = super::executable::launch(profile, workspace)
                .and_then(|launch| Client::start(&launch, &root, cancel, &entry.stop, generation));
            match started {
                Ok(client) => {
                    state.client = Some(client);
                    state.status = "ready".into();
                    state.detail.clear();
                    self.start_idle_pump(&entry, state.generation);
                }
                Err(error) => {
                    if error.starts_with("Executable not found:") {
                        state.status = "not_found".into();
                        state.detail = error.clone();
                        return self.fail(&mut outcome, "not_found", &error);
                    }
                    if cancel.requested()
                        || error.contains("cancelled")
                        || entry.stop.requested()
                        || error.contains("settings changed")
                    {
                        state.status = if entry.stop.requested() {
                            "settings_changed"
                        } else {
                            "cancelled"
                        }
                        .into();
                        state.detail = error.clone();
                        return self.fail(&mut outcome, &state.status, &error);
                    }
                    if error.contains("timed out") {
                        state.status = "timed_out".into();
                        state.detail = error.clone();
                        return self.fail(&mut outcome, "timed_out", &error);
                    }
                    state.crashed(&error);
                    return self.fail(&mut outcome, "crashed", &error);
                }
            }
        }
        outcome.instance_generation = Some(state.generation);
        let client = state.client.as_mut().expect("started client");
        let language = profile.language_for(&path).expect("matched path");
        let result = (|| {
            client.drain().map_err(QueryFailure::Transport)?;
            client
                .poll_open_documents()
                .map_err(QueryFailure::classify_local)?;
            let document = client
                .reconcile(&path, language)
                .map_err(QueryFailure::classify_local)?;
            client.drain().map_err(QueryFailure::Transport)?;
            outcome.document_version = Some(document.version);
            if client.sync_gap {
                outcome.freshness = "sync_gap".into();
            }
            match request {
                Request::Definition { line, column, .. }
                | Request::References { line, column, .. } => {
                    let method = if matches!(request, Request::Definition { .. }) {
                        "textDocument/definition"
                    } else {
                        "textDocument/references"
                    };
                    if !client.supports(method) {
                        outcome.status = "unsupported".into();
                        outcome.detail = format!("Server does not advertise {method}");
                        return Ok(());
                    }
                    let position = document
                        .model_position(line as usize, column as usize, client.encoding)
                        .map_err(|error| QueryFailure::Controlled("invalid_input", error))?;
                    let mut params = vec![
                        (
                            "textDocument".to_owned(),
                            Value::object([("uri", Value::string(&document.uri))]),
                        ),
                        ("position".to_owned(), position),
                    ];
                    if let Request::References {
                        include_declaration,
                        ..
                    } = request
                    {
                        params.push((
                            "context".into(),
                            Value::object([(
                                "includeDeclaration",
                                Value::Bool(include_declaration),
                            )]),
                        ));
                    }
                    let result = client
                        .query(
                            method,
                            Value::Object(params.into_iter().collect()),
                            Duration::from_millis(profile.request_timeout_ms),
                            cancel,
                            &entry.stop,
                        )
                        .map_err(QueryFailure::classify_protocol)?;
                    if client.partial_truncated {
                        outcome.truncated = true;
                        outcome.complete = false;
                    }
                    let values = result.get("result").ok_or_else(|| {
                        QueryFailure::Transport("LSP response has no result".into())
                    })?;
                    outcome.locations = locations::normalize(values, client.encoding, &document);
                    outcome.freshness = if client.sync_gap || client.sync_kind == 0 {
                        "sync_gap"
                    } else {
                        "synchronized"
                    }
                    .into();
                    outcome.detail = format!(
                        "{} location(s); indexing may still be in progress",
                        outcome.locations.len()
                    );
                }
                Request::Diagnostics { .. } => diagnostics(
                    client,
                    &document,
                    profile,
                    cancel,
                    &entry.stop,
                    &mut outcome,
                )
                .map_err(QueryFailure::classify_protocol)?,
                Request::Status { .. } => unreachable!(),
            }
            Ok::<(), QueryFailure>(())
        })();
        match result {
            // A completed request proves the instance healthy again.
            Ok(()) => state.failures = 0,
            Err(QueryFailure::Controlled(status, detail)) => {
                outcome.status = status.into();
                outcome.complete = false;
                outcome.detail = detail;
            }
            Err(QueryFailure::Transport(error)) => {
                state.crashed(&error);
                outcome.status = "crashed".into();
                outcome.complete = false;
                outcome.detail = error;
            }
        }
        if outcome.status == "ready"
            && let Some(client) = state.client.as_mut()
        {
            match client.poll_open_documents() {
                Ok(()) => {
                    if client.sync_gap {
                        outcome.freshness = "sync_gap".into();
                    }
                    if client
                        .docs
                        .values()
                        .find(|document| document.path == path)
                        .is_none_or(|document| Some(document.version) != outcome.document_version)
                    {
                        outcome.freshness = "stale".into();
                        outcome
                            .detail
                            .push_str("; workspace changed during the request");
                    }
                }
                Err(error) => {
                    outcome.freshness = "sync_gap".into();
                    outcome
                        .detail
                        .push_str(&format!("; workspace reconciliation failed: {error}"));
                }
            }
        }
        if entry.dirty.load(Ordering::Acquire) != dirty_before {
            outcome.freshness = "stale".into();
            outcome
                .detail
                .push_str("; files changed during the request");
        }
        if let Some(snapshot) = state
            .client
            .as_ref()
            .and_then(|client| client.docs.values().find(|doc| doc.path == path))
            && std::fs::read_to_string(&path).ok().as_deref() != Some(snapshot.text.as_str())
        {
            outcome.freshness = "stale".into();
            outcome
                .detail
                .push_str("; source document changed during the request");
        }
        let still_enabled = !entry.stop.requested()
            && self.authoritative(&profiles).ok().is_some_and(|current| {
                current.iter().any(|candidate| {
                    candidate.id == profile.id
                        && candidate.enabled
                        && candidate.instance_fingerprint() == entry.fingerprint
                })
            });
        if !still_enabled {
            self.retire(&entry);
            outcome.status = "settings_changed".into();
            outcome.freshness = "unavailable".into();
            outcome.complete = false;
            outcome.locations.clear();
            outcome.diagnostics.clear();
            outcome.detail = "LSP settings changed before the response completed".into();
        }
        outcome
    }

    fn fail(&self, outcome: &mut Outcome, status: &str, detail: &str) -> Outcome {
        outcome.status = status.into();
        outcome.detail = detail.into();
        outcome.complete = false;
        outcome.clone()
    }
}
