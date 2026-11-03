use super::{Client, Entry, Outcome, Runtime, State, Status};
use crate::cancel::Cancellation;
use crate::lsp::{Profile, Store};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

impl Runtime {
    pub fn include_credentials(&self, redactor: crate::redact::Redactor) {
        if let Ok(mut current) = self.0.redactor.lock() {
            *current = Some(redactor);
        }
    }
    pub fn status(&self, _workspace: &Path, profiles: &[Profile], server: Option<&str>) -> Outcome {
        let profiles = match self.authoritative(profiles) {
            Ok(v) => v,
            Err(e) => return Outcome::basic("configuration_error", e),
        };
        self.prune(&profiles);
        let entries = self.statuses();
        let mut outcome = Outcome::basic("ready", "Configured LSP servers and observed instances");
        outcome.servers = profiles
            .iter()
            .filter(|p| server.is_none_or(|id| p.id == id))
            .flat_map(|p| {
                let observed = entries
                    .iter()
                    .filter(|e| e.server == p.id)
                    .cloned()
                    .collect::<Vec<_>>();
                if observed.is_empty() {
                    vec![Status {
                        server: p.id.clone(),
                        root: None,
                        state: if p.enabled { "not_started" } else { "off" }.into(),
                        detail: if p.enabled {
                            "No conversation instance started"
                        } else {
                            "Disabled"
                        }
                        .into(),
                        capabilities: None,
                        pid: None,
                        generation: None,
                    }]
                } else {
                    observed
                }
            })
            .collect();
        if server.is_some() && outcome.servers.is_empty() {
            outcome.status = "unavailable".into();
            outcome.detail = "Unknown LSP server ID".into();
        }
        outcome
    }

    pub fn statuses(&self) -> Vec<Status> {
        let Ok(entries) = self.0.entries.lock() else {
            return vec![];
        };
        entries
            .values()
            .map(|entry| {
                if let Ok(state) = entry.state.try_lock() {
                    Status {
                        server: entry.id.clone(),
                        root: Some(entry.root.clone()),
                        state: state.status.clone(),
                        detail: if state.detail.is_empty() {
                            entry.root_source.clone()
                        } else {
                            state.detail.clone()
                        },
                        capabilities: state.client.as_ref().map(|c| c.capabilities.clone()),
                        pid: state.client.as_ref().map(Client::pid),
                        generation: state.client.as_ref().map(|c| c.generation),
                    }
                } else {
                    Status {
                        server: entry.id.clone(),
                        root: Some(entry.root.clone()),
                        state: "busy".into(),
                        detail: "Request in progress".into(),
                        capabilities: None,
                        pid: None,
                        generation: None,
                    }
                }
            })
            .collect()
    }

    pub fn invalidate_paths(&self, paths: &[PathBuf]) {
        if let Ok(entries) = self.0.entries.lock() {
            for entry in entries.values() {
                if paths.is_empty() || paths.iter().any(|p| p.starts_with(&entry.root)) {
                    entry.dirty.fetch_add(1, Ordering::AcqRel);
                }
            }
        }
    }

    pub fn configuration_changed(&self, id: &str) {
        let removed = if let Ok(mut entries) = self.0.entries.lock() {
            let mut removed = Vec::new();
            entries.retain(|(server, _), entry| {
                if server == id {
                    entry.stop.cancel();
                    removed.push(Arc::clone(entry));
                    false
                } else {
                    true
                }
            });
            removed
        } else {
            Vec::new()
        };
        self.stop_entries(removed);
    }

    pub fn reset_restart(&self, id: &str) {
        if let Ok(entries) = self.0.entries.lock() {
            for entry in entries.values().filter(|e| e.id == id) {
                if let Ok(mut state) = entry.state.try_lock() {
                    state.failures = 0;
                    state.retry_at = None;
                    state.status = "not_started".into();
                }
            }
        }
    }

    pub fn shutdown(&self) {
        let removed = if let Ok(mut entries) = self.0.entries.lock() {
            for entry in entries.values() {
                entry.stop.cancel();
            }
            entries.drain().map(|(_, entry)| entry).collect()
        } else {
            Vec::new()
        };
        self.stop_entries(removed);
    }

    pub(super) fn authoritative(&self, fallback: &[Profile]) -> Result<Vec<Profile>, String> {
        let profiles = if let Some(home) = &self.0.home {
            Store::new(home.clone()).load()
        } else {
            Ok(fallback.to_vec())
        }?;
        if let Ok(current) = self.0.redactor.lock()
            && let Some(redactor) = current.as_ref()
        {
            for profile in &profiles {
                for secret in profile.environment.values() {
                    redactor.add_secret(secret.clone());
                }
            }
        }
        Ok(profiles)
    }
    pub(super) fn prune(&self, profiles: &[Profile]) {
        let removed = if let Ok(mut entries) = self.0.entries.lock() {
            let mut removed = Vec::new();
            entries.retain(|(id, _), entry| {
                let valid = profiles.iter().any(|p| {
                    &p.id == id && p.enabled && p.instance_fingerprint() == entry.fingerprint
                });
                if !valid {
                    entry.stop.cancel();
                    removed.push(Arc::clone(entry));
                }
                valid
            });
            removed
        } else {
            Vec::new()
        };
        self.stop_entries(removed);
    }
    fn stop_entries(&self, entries: Vec<Arc<Entry>>) {
        for entry in entries {
            if let Ok(mut state) = entry.state.lock()
                && let Some(mut client) = state.client.take()
            {
                client.shutdown();
            }
        }
    }
    pub(super) fn entry(&self, profile: &Profile, root: &Path, root_source: &str) -> Arc<Entry> {
        let mut entries = self.0.entries.lock().expect("LSP registry");
        entries
            .entry((profile.id.clone(), root.to_path_buf()))
            .or_insert_with(|| {
                Arc::new(Entry {
                    id: profile.id.clone(),
                    root: root.to_path_buf(),
                    root_source: root_source.into(),
                    fingerprint: profile.instance_fingerprint(),
                    stop: Cancellation::default(),
                    dirty: AtomicU64::new(0),
                    state: Mutex::new(State {
                        client: None,
                        generation: 0,
                        failures: 0,
                        retry_at: None,
                        status: "not_started".into(),
                        detail: String::new(),
                    }),
                })
            })
            .clone()
    }
}
