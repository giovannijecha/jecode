//! Points: named moments whose workspace state can be restored. A point is
//! preceded by bringing the archive up to date with the workspace.
use super::record::Record;
use super::scan::{self, Summary};
use super::store::Transaction;
use super::workspace::{Readiness, Watching, Workspace};
use std::time::{Duration, Instant};

const QUIET: Duration = Duration::from_millis(30);
/// The helper's marker echo is not ordered with file notifications, whose
/// delivery took up to 6.4 ms in the lab; command finish waits at least this long.
const NOTICE: Duration = Duration::from_millis(10);
const SETTLE_LIMIT: Duration = Duration::from_millis(300);
/// Condition variable timeouts on Windows overran a 10 ms wait by up to 9 ms;
/// `thread::sleep` uses a high-resolution timer, so `settle` polls.
const POLL: Duration = Duration::from_millis(1);

impl Workspace {
    /// A different process can change coverage without changing scope rules.
    /// Publish that fact before a model request or a command uses the archive.
    pub(super) fn observe_coverage(&self, store: &super::store::Store) {
        if store.index.recovery.mode.is_some() {
            self.signals.update(|health| {
                health.selective =
                    Some("on-demand recovery; only captured paths are protected".into())
            });
            return;
        }
        let Some((complete, reason)) = store.index.current_coverage() else {
            return;
        };
        let selective = if *complete {
            None
        } else {
            reason.strip_prefix("selective: ")
        };
        self.signals.update(|health| {
            if let Some(reason) = selective {
                health.selective = Some(reason.into());
            } else if health.failure.is_none() && !matches!(health.readiness, Readiness::Failed(_))
            {
                health.selective = None;
            }
        });
    }

    /// Drains reported changes, or scans without notifications, then records
    /// a point. A watched point can precede a late external notification. With
    /// `reuse`, an unchanged observed state keeps its last point.
    pub fn mark(
        &self,
        kind: &str,
        label: &str,
        origin: &str,
        pinned: bool,
        reuse: bool,
    ) -> Result<u64, String> {
        super::lifecycle::touch_throttled(&self.archive)?;
        // The scan may rematerialize expired current heads. Keep collection
        // from dropping them before the milestone names their exact bytes.
        let _gc = if kind == "milestone" {
            Some(
                super::locks::Exclusive::acquire_cancellable(
                    &super::layout::gc_lock(&self.archive),
                    &self.cancellation,
                )
                .map_err(|error| format!("Could not coordinate checkpoint milestone: {error}"))?,
            )
        } else {
            None
        };
        let summary = if kind == "milestone" {
            // A named point must materialize tracked metadata-only heads.
            // Targeted mode still visits only the paths already protected.
            Some(scan::scan(self, "external", true, &self.cancellation)?)
        } else {
            self.catch_up("external")?
        };
        let reused = {
            let store = self.store();
            let index = &store.index;
            index
                .last_point()
                .filter(|point| {
                    reuse
                        && point.kind != super::recovery::GROUP_KIND
                        && point.kind != super::recovery::SNAPSHOT_KIND
                        && index.last_change < point.seq
                        && (index.coverage_at(point.seq) || !index.coverage_complete())
                })
                .map(|point| point.seq)
        };
        if let Some(seq) = reused {
            // A reused point can name frames another process wrote without a
            // flush. It must be stable before a command can change the files.
            self.store().transaction(true, |_| Ok(()))?;
            return Ok(seq);
        }
        #[cfg(test)]
        if kind == "milestone" {
            super::tests::process::stage("milestone-captured");
        }
        let point = Some((kind, label, origin, pinned));
        let seq = match summary {
            Some(summary) => self.covered_point(&summary, point),
            None => self.store().transaction(false, |transaction| {
                Ok(Some(transaction.point(kind, label, origin, pinned)))
            }),
        }?;
        // This also persists the scan batches that preceded the point.
        self.store().transaction(true, |_| Ok(()))?;
        Ok(seq.unwrap_or_default())
    }

    /// Records complete coverage after a full scan, optionally with a point.
    /// Without notifications, coverage is complete only at that instant.
    pub(super) fn covered_point(
        &self,
        summary: &Summary,
        point: Option<(&str, &str, &str, bool)>,
    ) -> Result<Option<u64>, String> {
        let complete =
            summary.unreadable == 0 && summary.unsupported == 0 && summary.limited.is_none();
        let polling = self.polling();
        self.store().transaction(false, |transaction| {
            let coverage = |transaction: &mut Transaction<'_>, complete: bool, reason: &str| {
                if transaction.index().coverage_complete() != complete
                    || transaction.index().current_coverage().is_none()
                {
                    let seq = transaction.seq();
                    transaction.record(Record::Coverage {
                        seq,
                        complete,
                        reason: reason.into(),
                    });
                }
            };
            if complete {
                coverage(transaction, true, "scan");
            } else if let Some(reason) = &summary.limited {
                // Persist the selected scope across process restarts. A
                // later full verification can establish full coverage again.
                let reason = format!("selective: {reason}");
                if transaction
                    .index()
                    .coverage
                    .values()
                    .next_back()
                    .is_none_or(|(_, old)| old != &reason)
                    || transaction.index().current_coverage().is_none()
                {
                    let seq = transaction.seq();
                    transaction.record(Record::Coverage {
                        seq,
                        complete: false,
                        reason,
                    });
                }
            } else {
                coverage(
                    transaction,
                    false,
                    &format!(
                        "{} unreadable directories, {} unsupported names",
                        summary.unreadable, summary.unsupported
                    ),
                );
            }
            let seq = point.map(|(kind, label, origin, pinned)| {
                transaction.point(kind, label, origin, pinned)
            });
            if polling && complete {
                // Coverage(true) above is still buffered, so consulting the
                // old index here can skip this invalidation. The point names
                // the complete observation; later unobserved history does not.
                let seq = transaction.seq();
                transaction.record(Record::Coverage {
                    seq,
                    complete: false,
                    reason: "not watched".into(),
                });
            }
            Ok(seq)
        })
    }

    /// Captures changes already reported before a point, or scans without
    /// notifications. A watched point records the observed state; a future
    /// notification can still report an earlier external filesystem change.
    pub(super) fn catch_up(&self, origin: &str) -> Result<Option<Summary>, String> {
        self.catch_up_cancellable(origin, &self.cancellation)
    }

    pub(super) fn catch_up_cancellable(
        &self,
        origin: &str,
        cancellation: &crate::cancel::Cancellation,
    ) -> Result<Option<Summary>, String> {
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        let readiness = self.readiness();
        if !matches!(readiness, Readiness::Ready | Readiness::Failed(_)) {
            return Ok(None);
        }
        if matches!(readiness, Readiness::Failed(_)) {
            // A later successful full scan can recover an interrupted baseline
            // or offline reconciliation. Keep the failure visible until the
            // scan and its coverage record both succeed.
            let summary = scan::scan_full(self, origin, true, cancellation)?;
            if !self.polling() {
                self.settle();
            }
            self.flush(origin)?;
            self.covered_point(&summary, None)?;
            self.signals
                .update(|health| health.readiness = Readiness::Ready);
            return Ok(Some(summary));
        }
        if self.polling() {
            let summary = scan::scan(self, origin, false, cancellation)?;
            // Reported paths are reread even when their metadata matches.
            self.flush(origin)?;
            return Ok(Some(summary));
        }
        // Notifications never report files that only came into scope.
        let policy = {
            let mut store = self.store_cancellable(cancellation)?;
            store.refresh_cancellable(cancellation)?;
            self.observe_coverage(&store);
            store.policy.clone()
        };
        let scope = policy.id();
        self.signals.update(|health| {
            if health.scope != scope {
                health.scope = scope;
                health.selective = None;
                #[cfg(windows)]
                {
                    health.policy = policy.clone();
                }
                health.rescan = Some("the checkpoint scope changed".into());
            }
        });
        self.flush(origin)?;
        Ok(None)
    }

    /// Waits briefly for likely notifications of a completed command. The
    /// helper's marker tests its responsiveness but cannot order callbacks.
    pub(super) fn settle(&self) {
        let started = Instant::now();
        #[cfg(windows)]
        let marker = self
            .watcher
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
            .and_then(|watcher| watcher.marker());
        #[cfg(not(windows))]
        let marker: Option<u64> = None;
        loop {
            let elapsed = started.elapsed();
            {
                let health = self.signals.lock();
                let echoed = marker.is_none_or(|marker| health.marker >= marker);
                let quiet = health.event.is_none_or(|event| event.elapsed() >= QUIET);
                if (echoed && quiet && elapsed >= NOTICE)
                    || elapsed >= SETTLE_LIMIT
                    || health.watching != Watching::Active
                {
                    return;
                }
            }
            std::thread::sleep(POLL.min(SETTLE_LIMIT - elapsed));
        }
    }
}
