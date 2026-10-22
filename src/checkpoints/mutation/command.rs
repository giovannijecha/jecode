//! An explicit Bash preimage and scoped observation after execution.
use super::Freshness;
use crate::cancel::Cancellation;
use crate::checkpoints::scopes::{self, Captured};
use crate::checkpoints::workspace::Workspace;
use std::sync::Arc;

pub struct Command {
    workspace: Arc<Workspace>,
    origin: String,
    captured: Result<Captured, String>,
    /// Compatibility with callers that display a point before launch.
    pub point: Result<u64, String>,
    pub freshness: Freshness,
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
    pub fn finish(&self) -> Result<(), String> {
        if let Some(group) = self.captured.group {
            let mut store = self.workspace.store();
            store.transaction(true, |transaction| {
                crate::checkpoints::recovery::touch(transaction, group, &self.origin)
            })?;
        }
        let report = scopes::reconcile(
            &self.workspace,
            &self.captured,
            "external",
            &Cancellation::default(),
        )?;
        if !report.partial.is_empty() {
            let (path, reason) = &report.partial[0];
            return Err(format!(
                "{} declared paths could not be observed after the process ended; first: {path}: {reason}",
                report.partial.len(),
            ));
        }
        Ok(())
    }

    pub fn renew(&self) -> Result<(), String> {
        if let Some(group) = self.captured.group {
            let mut store = self.workspace.store();
            store.transaction(true, |transaction| {
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
                        &Cancellation::default(),
                    ) {
                        Ok(after) => report.partial.extend(after.partial),
                        Err(error) => report.error = Some(error),
                    }
                }
                if let Some(before) = captured.snapshot {
                    let mut store = self.workspace.store();
                    if let Err(error) = store.refresh() {
                        report.error.get_or_insert(error);
                    } else {
                        let index = &store.index;
                        report.after = Some(index.latest());
                        let mut changed: Vec<String> = index
                            .changed_between(captured.frontier.max(before), index.latest())
                            .into_iter()
                            .map(|version| version.path.clone())
                            .collect();
                        changed.sort();
                        changed.dedup();
                        report.changed = changed;
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

impl Drop for Command {
    fn drop(&mut self) {
        self.release();
    }
}
