use super::{Entry, Runtime};
use crate::lsp::Store;
use std::sync::Arc;
use std::time::{Duration, Instant};

impl Runtime {
    pub(super) fn start_idle_pump(&self, entry: &Arc<Entry>, generation: u64) {
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
                            entry.stop.cancel();
                            if let Ok(mut state) = entry.state.lock() {
                                if let Some(mut client) = state.client.take() {
                                    client.shutdown();
                                }
                                state.status = "off".into();
                                state.detail = "Configuration changed".into();
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
                let result = if let Some(client) = state.client.as_mut() {
                    client.drain().and_then(|()| {
                        if poll {
                            client.poll_open_documents()
                        } else {
                            Ok(())
                        }
                    })
                } else {
                    break;
                };
                if poll {
                    next_poll = Instant::now() + Duration::from_millis(500);
                }
                if let Err(error) = result {
                    state.client.take();
                    state.failures = state.failures.saturating_add(1);
                    state.retry_at =
                        Some(Instant::now() + Duration::from_secs(1 << state.failures.min(3)));
                    state.status = if state.failures >= 3 {
                        "restart_suppressed"
                    } else {
                        "crashed"
                    }
                    .into();
                    state.detail = error;
                    break;
                }
            }
        });
    }
}
