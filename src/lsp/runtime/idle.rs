use super::{Entry, QueryFailure, Runtime};
use crate::lsp::Store;
use std::sync::Arc;
use std::time::{Duration, Instant};

impl Runtime {
    pub(super) fn start_idle_pump(&self, entry: &Arc<Entry>, generation: u64) {
        let runtime = Arc::downgrade(&self.0);
        let weak = Arc::downgrade(entry);
        let home = self.0.home.clone();
        std::thread::spawn(move || {
            let mut next_poll = Instant::now();
            let mut next_config = Instant::now();
            loop {
                std::thread::sleep(Duration::from_millis(50));
                let Some(entry) = weak.upgrade() else { break };
                if entry.stop.requested() {
                    break;
                }
                if Instant::now() >= next_config {
                    next_config = Instant::now() + Duration::from_millis(250);
                    if let Some(home) = &home {
                        let valid = Store::new(home.clone())
                            .load()
                            .ok()
                            .is_some_and(|profiles| {
                                profiles.iter().any(|p| {
                                    p.id == entry.id
                                        && p.enabled
                                        && p.instance_fingerprint() == entry.fingerprint
                                })
                            });
                        if !valid {
                            if let Some(inner) = runtime.upgrade() {
                                Runtime(inner).retire(&entry);
                            }
                            break;
                        }
                    }
                }
                let Ok(mut state) = entry.state.try_lock() else {
                    continue;
                };
                if state.generation != generation {
                    break;
                }
                let poll = Instant::now() >= next_poll;
                let Some(client) = state.client.as_mut() else {
                    break;
                };
                let failure = match client.drain() {
                    Err(error) => Some(error),
                    // Local document problems are reported to the next request, not counted as crashes.
                    Ok(()) if poll => match client
                        .poll_open_documents()
                        .map_err(QueryFailure::classify_local)
                    {
                        Err(QueryFailure::Transport(error)) => Some(error),
                        _ => None,
                    },
                    Ok(()) => None,
                };
                if poll {
                    next_poll = Instant::now() + Duration::from_millis(500);
                }
                if let Some(error) = failure {
                    state.crashed(&error);
                    break;
                }
            }
        });
    }
}
