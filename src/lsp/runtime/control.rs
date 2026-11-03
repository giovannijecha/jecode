use super::{Client, Entry, Inner, Outcome, Runtime, State, Status};
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
        self.retire_where(|entry| entry.id == id);
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

    /// Stops every instance; the last runtime owner waits for them to exit.
    pub fn shutdown(&self) {
        self.retire_where(|_| true);
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
                profile.protect(redactor);
            }
        }
        Ok(profiles)
    }
    pub(super) fn prune(&self, profiles: &[Profile]) {
        self.retire_where(|entry| {
            entry.stop.requested()
                || !profiles.iter().any(|p| {
                    p.id == entry.id && p.enabled && p.instance_fingerprint() == entry.fingerprint
                })
        });
    }

    /// The registered instance for `profile` and `root`, replacing one that
    /// is stopping or was started with other settings.
    pub(super) fn entry(&self, profile: &Profile, root: &Path, root_source: &str) -> Arc<Entry> {
        let fingerprint = profile.instance_fingerprint();
        let key = (profile.id.clone(), root.to_path_buf());
        let mut entries = self.0.entries.lock().expect("LSP registry");
        if let Some(current) = entries.get(&key)
            && !current.stop.requested()
            && current.fingerprint == fingerprint
        {
            return Arc::clone(current);
        }
        let fresh = Arc::new(Entry {
            id: profile.id.clone(),
            root: root.to_path_buf(),
            root_source: root_source.into(),
            fingerprint,
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
        });
        let replaced = entries.insert(key, Arc::clone(&fresh));
        drop(entries);
        if let Some(replaced) = replaced {
            self.stop(replaced);
        }
        fresh
    }

    /// Unregisters `entry` unless a newer instance took its place, then stops it.
    pub(super) fn retire(&self, entry: &Arc<Entry>) {
        if let Ok(mut entries) = self.0.entries.lock() {
            let key = (entry.id.clone(), entry.root.clone());
            if entries
                .get(&key)
                .is_some_and(|current| Arc::ptr_eq(current, entry))
            {
                entries.remove(&key);
            }
        }
        self.stop(Arc::clone(entry));
    }

    fn retire_where(&self, stale: impl Fn(&Entry) -> bool) {
        let mut removed = Vec::new();
        if let Ok(mut entries) = self.0.entries.lock() {
            entries.retain(|_, entry| {
                let keep = !stale(entry);
                if !keep {
                    removed.push(Arc::clone(entry));
                }
                keep
            });
        }
        for entry in removed {
            self.stop(entry);
        }
    }

    /// Blocks new dispatch and cancels waiting work at once. The shutdown
    /// handshake waits for an in-flight request and for servers that ignore
    /// it, so it runs on its own thread instead of the caller's.
    fn stop(&self, entry: Arc<Entry>) {
        entry.stop.cancel();
        let worker = std::thread::spawn(move || {
            let client = entry
                .state
                .lock()
                .ok()
                .and_then(|mut state| state.client.take());
            if let Some(mut client) = client {
                client.shutdown();
            }
        });
        if let Ok(mut retiring) = self.0.retiring.lock() {
            retiring.retain(|worker| !worker.is_finished());
            retiring.push(worker);
        }
    }
}

impl Drop for Inner {
    /// The last owner, such as Jecode's exit, leaves no instance running.
    fn drop(&mut self) {
        let workers = match self.retiring.get_mut() {
            Ok(workers) => std::mem::take(workers),
            Err(poisoned) => std::mem::take(poisoned.into_inner()),
        };
        for worker in workers {
            let _ = worker.join();
        }
    }
}
