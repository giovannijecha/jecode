//! Bounded diagnostic caches and the per-document gaps their eviction leaves.
use super::{Client, DiagnosticReport};
use std::collections::HashMap;

const MAX_REPORTS: usize = 256;
const MAX_REPORT_BYTES: usize = 16 * 1024 * 1024;
const MAX_GAPS: usize = 4096;

/// Encoded size of each cached report, and documents whose push findings were evicted.
#[derive(Default)]
pub(super) struct Ledger {
    sizes: HashMap<(bool, String), usize>,
    gaps: HashMap<String, u64>,
    sequence: u64,
}

impl Ledger {
    fn mark_gap(&mut self, uri: String) {
        self.sequence += 1;
        self.gaps.insert(uri, self.sequence);
        if self.gaps.len() > MAX_GAPS {
            let floor = self.sequence - (MAX_GAPS / 2) as u64;
            self.gaps.retain(|_, sequence| *sequence > floor);
        }
    }
}

impl Client {
    /// Evicted pull reports are requested again by the next query; evicted push
    /// findings remain a gap for their document until the server pushes it again.
    pub(crate) fn put_report(&mut self, uri: String, report: DiagnosticReport, push: bool) {
        if push {
            self.report_ledger.gaps.remove(&uri);
            if report.values.is_empty() && !self.docs.contains_key(&uri) {
                // An empty report for an unmanaged file only clears what was there.
                self.push_diagnostics.remove(&uri);
                self.report_ledger.sizes.remove(&(true, uri));
                return;
            }
        }
        let size = report.values.iter().map(|value| value.encode().len()).sum();
        self.cache(push).insert(uri.clone(), report);
        let (push_cache, pull_cache) = (&self.push_diagnostics, &self.pull_diagnostics);
        let sizes = &mut self.report_ledger.sizes;
        sizes.insert((push, uri.clone()), size);
        // Reports removed elsewhere, such as for closed documents, leave stale sizes.
        sizes.retain(|(push, uri), _| {
            if *push {
                push_cache.contains_key(uri)
            } else {
                pull_cache.contains_key(uri)
            }
        });
        let count = push_cache.len() + pull_cache.len();
        if count <= MAX_REPORTS && sizes.values().sum::<usize>() <= MAX_REPORT_BYTES {
            return;
        }
        let retained = self.cache(push).remove(&uri);
        self.pull_diagnostics.clear();
        for (evicted, _) in self.push_diagnostics.drain() {
            self.report_ledger.mark_gap(evicted);
        }
        if let Some(report) = retained {
            self.cache(push).insert(uri.clone(), report);
        }
        self.report_ledger
            .sizes
            .retain(|key, _| key.0 == push && key.1 == uri);
    }

    /// Whether this document's push findings were evicted and not reported again.
    pub fn diagnostic_gap(&self, uri: &str) -> bool {
        self.report_ledger.gaps.contains_key(uri)
    }

    fn cache(&mut self, push: bool) -> &mut HashMap<String, DiagnosticReport> {
        if push {
            &mut self.push_diagnostics
        } else {
            &mut self.pull_diagnostics
        }
    }
}
