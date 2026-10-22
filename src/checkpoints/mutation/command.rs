//! An explicit Bash preimage and scoped observation after execution.
use super::Freshness;
use crate::cancel::Cancellation;
use crate::checkpoints::index::Index;
use crate::checkpoints::scopes::{self, Captured, Selection};
use crate::checkpoints::workspace::Workspace;
use std::collections::BTreeSet;
use std::sync::Arc;

pub struct Command {
    workspace: Arc<Workspace>,
    origin: String,
    captured: Result<Captured, String>,
    /// Compatibility with callers that display a point before launch.
    pub point: Result<u64, String>,
    pub freshness: Freshness,
    cancellation: Cancellation,
    finished: bool,
}

#[derive(Clone, Debug, Default)]
pub struct CommandReport {
    pub before: Option<u64>,
    pub group: Option<u64>,
    pub after: Option<u64>,
    pub changed: Vec<String>,
    pub error: Option<String>,
    pub saved: usize,
    pub partial: Vec<(String, String)>,
    pub complete_dirs: Vec<String>,
}

impl CommandReport {
    pub fn status(&self) -> &'static str {
        if self.saved == 0 && self.complete_dirs.is_empty() {
            "none"
        } else if self.error.is_some() || !self.partial.is_empty() {
            "partial"
        } else {
            "saved"
        }
    }
}

/// Retained by a persistent process until its monitor observes the final
/// exit. The monitor can finish observation even after the original tool turn.
#[derive(Clone)]
pub struct Completion {
    workspace: Arc<Workspace>,
    origin: String,
    captured: Captured,
}

impl Completion {
    #[cfg(test)]
    pub fn finish(&self) -> Result<(), String> {
        self.finish_cancellable(&Cancellation::default())
    }

    pub fn finish_cancellable(&self, cancellation: &Cancellation) -> Result<(), String> {
        let report = scopes::reconcile(&self.workspace, &self.captured, "external", cancellation)?;
        if !report.partial.is_empty() {
            let (path, reason) = &report.partial[0];
            return Err(format!(
                "{} declared paths could not be observed after the process ended; first: {path}: {reason}",
                report.partial.len(),
            ));
        }
        Ok(())
    }

    pub fn renew_cancellable(&self, cancellation: &Cancellation) -> Result<(), String> {
        if let Some(group) = self.captured.group {
            let mut store = self.workspace.store_cancellable(cancellation)?;
            store.transaction_cancellable(true, cancellation, |transaction| {
                crate::checkpoints::recovery::touch(transaction, group, &self.origin)
            })?;
        }
        Ok(())
    }
}

impl Workspace {
    /// The legacy test helper deliberately requests the whole workspace.
    #[cfg(test)]
    pub fn command(self: &Arc<Self>, origin: &str, text: &str) -> Command {
        self.command_scoped_cancellable(origin, text, &[".".into()], &Cancellation::default())
    }

    /// Compatibility for direct callers. Production Bash passes explicit
    /// paths through `command_scoped_cancellable`.
    #[cfg(test)]
    pub fn command_cancellable(
        self: &Arc<Self>,
        origin: &str,
        text: &str,
        cancellation: &Cancellation,
    ) -> Command {
        self.command_scoped_cancellable(origin, text, &[".".into()], cancellation)
    }

    pub fn command_scoped_cancellable(
        self: &Arc<Self>,
        origin: &str,
        _text: &str,
        paths: &[String],
        cancellation: &Cancellation,
    ) -> Command {
        self.signals
            .update(|health| health.bash.push(origin.to_string()));
        let captured = scopes::capture(self, paths, origin, cancellation);
        let point = captured
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|captured| captured.snapshot.ok_or("no paths protected".into()));
        let freshness = match &captured {
            Ok(captured) if captured.saved > 0 || !captured.complete_dirs.is_empty() => {
                if captured.partial.is_empty() {
                    Freshness::Current
                } else {
                    Freshness::Selective(format!(
                        "{} paths could not be protected",
                        captured.partial.len()
                    ))
                }
            }
            Ok(captured) if !captured.partial.is_empty() => Freshness::Unavailable(format!(
                "{} paths could not be protected",
                captured.partial.len()
            )),
            Ok(_) => Freshness::Unavailable("no paths protected".into()),
            Err(error) => Freshness::Unavailable(error.clone()),
        };
        Command {
            workspace: Arc::clone(self),
            origin: origin.into(),
            captured,
            point,
            freshness,
            cancellation: cancellation.clone(),
            finished: false,
        }
    }
}

impl Command {
    pub fn completion(&self) -> Option<Completion> {
        let captured = self.captured.as_ref().ok()?;
        if captured.selection.files.is_empty() && captured.selection.directories.is_empty() {
            return None;
        }
        Some(Completion {
            workspace: Arc::clone(&self.workspace),
            origin: self.origin.clone(),
            captured: captured.clone(),
        })
    }

    /// Captures changes already visible when Bash returns. A persistent
    /// process's completion separately observes its final changes.
    pub fn finish(mut self) -> CommandReport {
        let mut report = CommandReport::default();
        match &self.captured {
            Ok(captured) => {
                report.before = captured.snapshot;
                report.group = captured.group;
                report.saved = captured.saved;
                report.partial = captured.partial.clone();
                report.complete_dirs = captured.complete_dirs.clone();
                if !captured.selection.files.is_empty()
                    || !captured.selection.directories.is_empty()
                {
                    match scopes::reconcile(
                        &self.workspace,
                        captured,
                        "external",
                        &self.cancellation,
                    ) {
                        Ok(after) => report.partial.extend(after.partial),
                        Err(error) => report.error = Some(error),
                    }
                }
                if let Some(before) = captured.snapshot {
                    match self.workspace.store_cancellable(&self.cancellation) {
                        Ok(mut store) => {
                            if let Err(error) = store.refresh_cancellable(&self.cancellation) {
                                report.error.get_or_insert(error);
                            } else {
                                let index = &store.index;
                                if report.error.is_none() && report.partial.is_empty() {
                                    report.after = Some(index.latest());
                                }
                                report.changed = changed_in_selection(
                                    index,
                                    &captured.selection,
                                    captured.frontier.max(before),
                                );
                            }
                        }
                        Err(error) => {
                            report.error.get_or_insert(error);
                        }
                    }
                }
            }
            Err(error) => report.error = Some(error.clone()),
        }
        self.release();
        report
    }

    fn release(&mut self) {
        if std::mem::replace(&mut self.finished, true) {
            return;
        }
        let origin = &self.origin;
        self.workspace.signals.update(|health| {
            if let Some(position) = health.bash.iter().position(|running| running == origin) {
                health.bash.remove(position);
            }
        });
    }
}

fn changed_in_selection(index: &Index, selection: &Selection, after: u64) -> Vec<String> {
    let mut changed = BTreeSet::new();
    let changed_since = |versions: &[crate::checkpoints::record::Version]| {
        versions.last().is_some_and(|version| version.seq > after)
    };
    for name in &selection.files {
        if index
            .paths
            .get(name)
            .is_some_and(|versions| changed_since(versions))
        {
            changed.insert(name.clone());
        }
    }
    for root in &selection.directories {
        if root.is_empty() {
            for (name, versions) in &index.paths {
                if changed_since(versions) {
                    changed.insert(name.clone());
                }
            }
            continue;
        }
        if index
            .paths
            .get(root)
            .is_some_and(|versions| changed_since(versions))
        {
            changed.insert(root.clone());
        }
        let prefix = format!("{root}/");
        for (name, versions) in index.paths.range(prefix.clone()..) {
            if !name.starts_with(&prefix) {
                break;
            }
            if changed_since(versions) {
                changed.insert(name.clone());
            }
        }
    }
    changed.into_iter().collect()
}

impl Drop for Command {
    fn drop(&mut self) {
        self.release();
    }
}
