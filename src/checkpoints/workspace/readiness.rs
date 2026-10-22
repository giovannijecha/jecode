//! Baseline readiness and the coverage state exposed to tool callers.
use super::{Readiness, Watching, Workspace};
#[cfg(test)]
use crate::cancel::Cancellation;
#[cfg(test)]
use std::time::Duration;

impl Workspace {
    pub(in crate::checkpoints) fn progress(&self, examined: usize) {
        let mut health = self.signals.lock();
        match health.readiness {
            Readiness::Baseline(_) => health.readiness = Readiness::Baseline(examined),
            Readiness::Reconciling(_) => health.readiness = Readiness::Reconciling(examined),
            _ => {}
        }
    }

    pub fn readiness(&self) -> Readiness {
        self.signals.lock().readiness.clone()
    }

    /// Bash waits until the initial observation has finished. Waiting never
    /// holds the store lock, so the capture worker can make progress.
    #[cfg(test)]
    pub fn wait_ready(&self) -> Result<(), String> {
        self.wait_ready_cancellable(&self.cancellation)
    }

    #[cfg(test)]
    pub fn wait_ready_cancellable(&self, cancellation: &Cancellation) -> Result<(), String> {
        let mut health = self.signals.lock();
        loop {
            if cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            match &health.readiness {
                Readiness::Ready => break,
                Readiness::Failed(_) => {
                    // A failed initial capture may have hit a transient I/O
                    // error. Retry the full scan before declaring Bash
                    // unprotected, as ordinary points already do.
                    drop(health);
                    self.catch_up_cancellable("external", cancellation)?;
                    health = self.signals.lock();
                }
                _ if !self.options.background => {
                    return Err("checkpoint capture has not been initialized".into());
                }
                _ if health.stopped => return Err("checkpoint capture stopped".into()),
                _ => {
                    health = self
                        .signals
                        .changed
                        .wait_timeout(health, Duration::from_millis(50))
                        .unwrap_or_else(|error| error.into_inner())
                        .0;
                }
            }
        }
        drop(health);
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        let mut store = self.store_cancellable(cancellation)?;
        store.refresh_cancellable(cancellation)?;
        self.observe_coverage(&store);
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        match store.index.current_coverage() {
            Some((true, _)) => Ok(()),
            // Polling records a complete scan followed by this invalidation:
            // it cannot claim continuous observation between points.
            Some((false, reason)) if reason == "not watched" => Ok(()),
            Some((false, reason)) => Err(format!("checkpoint coverage is incomplete: {reason}")),
            None => Err("checkpoint coverage has not been established".into()),
        }
    }

    pub fn selective(&self) -> Option<String> {
        self.signals.lock().selective.clone()
    }

    pub(in crate::checkpoints) fn limit_scope(&self, reason: &str) -> Result<(), String> {
        self.signals
            .update(|health| health.selective = Some(reason.into()));
        super::scan::coverage(&mut self.store(), false, &format!("selective: {reason}"))
    }

    #[cfg(test)]
    pub fn watching(&self) -> Watching {
        self.signals.lock().watching.clone()
    }

    /// Without live notifications, freshness comes from scans.
    pub fn polling(&self) -> bool {
        self.signals.lock().watching != Watching::Active
    }
}
