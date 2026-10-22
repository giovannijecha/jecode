//! A conversation's use of the workspace archive: protection for its tool
//! calls, turn points, notices about history changes made elsewhere and its
//! entry in the activity list. Opening never fails; an unavailable archive is
//! reported on every protected call instead of blocking it.
use super::activity::{self, Registration};
use super::index::{EVENT_LIMIT, reference};
pub use super::mutation::Completion;
use super::mutation::{Command, Freshness, Mutation, Protection};
use super::workspace::{self, Options, Workspace};
use crate::json::Value;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

const CHANGED_LIMIT: usize = 20;
const NOTICE_LIMIT: usize = 20;
const NOTICE_LINE: usize = 400;
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

pub struct Session {
    id: u64,
    workspace: Result<Arc<Workspace>, String>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    origin: String,
    cursor: u64,
    activity: Option<Registration>,
    /// A restore by this conversation changed JECODE.md.
    instructions: bool,
}

/// History changes the model has not seen yet.
pub struct Notice {
    pub cursor: u64,
    pub text: String,
    /// JECODE.md changed; project instructions must be reloaded.
    pub instructions: bool,
}

impl Session {
    pub fn open(home: &Path, root: &Path) -> Arc<Self> {
        Self::open_with(home, root, Options::interactive())
    }

    pub fn open_with(home: &Path, root: &Path, options: Options) -> Arc<Self> {
        Arc::new(Self {
            id: NEXT_SESSION.fetch_add(1, Ordering::Relaxed),
            workspace: workspace::open(home, root, options)
                .map_err(|error| format!("checkpoints are unavailable: {error}")),
            state: Mutex::default(),
        })
    }

    /// Opens without a watcher, worker or baseline scan.
    #[cfg(test)]
    pub fn offline(home: &Path, root: &Path) -> Arc<Self> {
        Self::open_with(home, root, Options::offline())
    }

    pub fn workspace(&self) -> Result<&Arc<Workspace>, String> {
        self.workspace.as_ref().map_err(Clone::clone)
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn origin(&self) -> String {
        self.state().origin.clone()
    }

    /// The history observed by the most recent notice check, including checks
    /// with no new events. Persisted independently of workspace points.
    pub fn cursor(&self) -> u64 {
        self.state().cursor
    }

    /// Starts or resumes conversation `origin`. Notices start after `cursor`,
    /// else after the conversation's last point, else now.
    pub fn begin(&self, origin: &str, cursor: Option<u64>) {
        let (cursor, activity) = match self.workspace() {
            Ok(workspace) => {
                let mut store = workspace.store();
                let _ = store.refresh();
                let index = &store.index;
                let own = index
                    .points
                    .values()
                    .rev()
                    .find(|point| point.origin == origin)
                    .map(|point| point.seq);
                let cursor = cursor.or(own).unwrap_or_else(|| index.latest());
                drop(store);
                (cursor, activity::register(&workspace.archive, origin).ok())
            }
            Err(_) => (0, None),
        };
        *self.state() = State {
            origin: origin.into(),
            cursor,
            activity,
            instructions: false,
        };
    }

    /// Describes what this conversation is doing to other sessions.
    pub fn activity(&self, state: &str) {
        if let Some(activity) = &self.state().activity {
            activity.update(state);
        }
    }

    /// Publish activity and observe archive metadata without scanning files.
    pub fn turn(&self, _label: &str) -> Result<u64, String> {
        let workspace = self.workspace()?;
        self.activity("working");
        let mut store = workspace.store();
        store.refresh()?;
        Ok(store.index.latest())
    }

    /// Notes that this conversation's own restore changed JECODE.md.
    pub fn restored_instructions(&self) {
        self.state().instructions = true;
    }

    /// History changes recorded by other conversations, the user or recovery
    /// since the last notice.
    pub fn notices(&self) -> Option<Notice> {
        let workspace = self.workspace().ok()?;
        let mut store = workspace.store();
        let _ = store.refresh();
        workspace.observe_coverage(&store);
        let index = &store.index;
        let mut state = self.state();
        let cursor = state.cursor;
        let gap = index.events.len() == EVENT_LIMIT
            && index.events.front().is_some_and(|event| event.seq > cursor);
        let events: Vec<_> = index
            .events
            .iter()
            .filter(|event| event.seq > cursor && event.origin != state.origin)
            .collect();
        let own_instructions = std::mem::take(&mut state.instructions);
        let instructions = gap
            || own_instructions
            || events
                .iter()
                .any(|event| event.paths.iter().any(|path| instructions_path(path)));
        state.cursor = cursor.max(index.latest());
        if events.is_empty() && !instructions {
            return None;
        }
        let own = state.origin.as_str();
        let mut lines: Vec<String> = events
            .iter()
            .skip(events.len().saturating_sub(NOTICE_LIMIT))
            .map(|event| {
                let mut line = format!(
                    "{} by {}: {}",
                    reference(event.seq),
                    author(&event.origin, own),
                    event.text
                );
                if !event.paths.is_empty() {
                    let shown: Vec<&str> =
                        event.paths.iter().take(10).map(String::as_str).collect();
                    let more = event.paths.len().saturating_sub(shown.len());
                    line.push_str(&format!(
                        " ({}{})",
                        shown.join(", "),
                        if more > 0 {
                            format!(", +{more} more")
                        } else {
                            String::new()
                        }
                    ));
                }
                if line.chars().count() > NOTICE_LINE {
                    format!("{}...", line.chars().take(NOTICE_LINE).collect::<String>())
                } else {
                    line
                }
            })
            .collect();
        if events.len() > NOTICE_LIMIT {
            lines.insert(
                0,
                format!(
                    "{} earlier events omitted; inspect checkpoint history for details.",
                    events.len() - NOTICE_LIMIT
                ),
            );
        }
        if gap {
            lines.insert(0, "Older workspace notices may have been released; inspect current files before relying on earlier context.".into());
        }
        if instructions {
            lines.push("JECODE.md changed; the project instructions were reloaded.".into());
        }
        Some(Notice {
            cursor: state.cursor,
            text: lines.join("\n"),
            instructions,
        })
    }

    /// Protects one file change by a tool.
    #[cfg(test)]
    pub fn file(&self, kind: &'static str, path: &Path) -> FileGuard<'_> {
        self.file_cancellable(kind, path, &crate::cancel::Cancellation::default())
    }

    /// Protects one file change while honoring interruption during preparation.
    pub fn file_cancellable(
        &self,
        kind: &'static str,
        path: &Path,
        cancellation: &crate::cancel::Cancellation,
    ) -> FileGuard<'_> {
        let origin = self.origin();
        FileGuard {
            mutation: self
                .workspace()
                .map(|workspace| workspace.mutation_cancellable(&origin, kind, path, cancellation)),
        }
    }

    /// Captures declared paths before a shell command.
    pub fn command_scoped_cancellable(
        &self,
        text: &str,
        paths: &[String],
        cancellation: &crate::cancel::Cancellation,
    ) -> CommandGuard {
        let label: String = text.chars().take(80).collect();
        self.activity(&format!("running: {label}"));
        CommandGuard {
            command: self.workspace().map(|workspace| {
                workspace.command_scoped_cancellable(&self.origin(), text, paths, cancellation)
            }),
        }
    }

    /// Running background processes started by this conversation.
    pub fn set_background(&self, running: usize) {
        if let Ok(workspace) = self.workspace() {
            workspace.set_background(self.id, running);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Ok(workspace) = &self.workspace {
            workspace.set_background(self.id, 0);
        }
    }
}

fn instructions_path(path: &str) -> bool {
    path.rsplit('/').next() == Some("JECODE.md")
}

/// Who recorded an event, as the model should read it.
pub(super) fn author(origin: &str, own: &str) -> String {
    match origin.strip_prefix("user:") {
        Some(session) if session == own => "the user in this conversation".into(),
        Some(_) => "the user in another conversation".into(),
        None if origin == "cli" => "the user (jecode checkpoints)".into(),
        None if origin == "recovery" => "recovery".into(),
        None if origin == "policy" => "the user (policy.json)".into(),
        None => format!("another conversation ({origin})"),
    }
}

pub struct FileGuard<'a> {
    mutation: Result<Mutation<'a>, String>,
}

impl FileGuard<'_> {
    pub fn check_cancelled(&self) -> Result<(), String> {
        if let Ok(mutation) = &self.mutation {
            mutation.check_cancelled()?;
        }
        Ok(())
    }

    /// Durably saves the previous state and the intended content.
    pub fn prepare(&mut self, content: &[u8]) {
        if let Ok(mutation) = &mut self.mutation {
            mutation.prepare(content);
        }
    }

    /// Saves an intended replacement before its source is staged on disk.
    pub fn prepare_reader(&mut self, reader: impl std::io::Read, length: u64) {
        if let Ok(mutation) = &mut self.mutation {
            mutation.prepare_reader(reader, length);
        }
    }

    /// Returns the exact previous text read while preparing a write, when it
    /// fits the requested diff limit. `Some(None)` means the path was absent.
    pub fn prepare_with_previous(
        &mut self,
        content: &[u8],
        max_text_bytes: usize,
    ) -> Option<Option<String>> {
        self.mutation
            .as_mut()
            .ok()?
            .prepare_with_previous(content, max_text_bytes)
    }

    /// Records the result and describes the protection for the tool result.
    pub fn finish(self, applied: bool) -> Value {
        match self.mutation {
            Ok(mutation) => protection(&mutation.finish(applied)),
            Err(reason) => unprotected(&reason),
        }
    }
}

pub fn protection(protection: &Protection) -> Value {
    match protection {
        Protection::Protected { before, after } => {
            let mut fields = vec![
                ("status", Value::string("saved")),
                ("before", Value::string(reference(*before))),
            ];
            if let Some(after) = after {
                fields.push(("after", Value::string(reference(*after))));
            }
            object(fields)
        }
        Protection::Excluded(reason) => Value::object([
            ("status", Value::string("not_tracked")),
            ("reason", Value::string(reason)),
        ]),
        Protection::Unprotected(reason) => unprotected(reason),
    }
}

fn unprotected(reason: &str) -> Value {
    Value::object([
        ("status", Value::string("unprotected")),
        (
            "reason",
            Value::string(format!("the previous state was not saved: {reason}")),
        ),
    ])
}

pub struct CommandGuard {
    command: Result<Command, String>,
}

#[cfg(test)]
mod cancellation_tests;

impl CommandGuard {
    pub fn completion(&self) -> Option<Completion> {
        self.command.as_ref().ok()?.completion()
    }

    /// One line describing the point before the command, for display.
    pub fn note(&self) -> String {
        match &self.command {
            Err(reason) => format!("checkpoint none · {reason}"),
            Ok(command) => match (&command.point, &command.freshness) {
                (Ok(point), Freshness::Current) => format!("checkpoint {}", reference(*point)),
                (Ok(point), Freshness::Selective(_)) => {
                    format!("checkpoint {} · partial protection", reference(*point))
                }
                (_, Freshness::Unavailable(reason)) | (Err(reason), _) => {
                    format!("checkpoint none · {reason}")
                }
            },
        }
    }

    /// Captures the command's changes; `running` means it continues in the background.
    pub fn finish(self, running: bool) -> Value {
        let command = match self.command {
            Ok(command) => command,
            Err(reason) => {
                return Value::object([
                    ("status", Value::string("none")),
                    ("reason", Value::string(reason)),
                ]);
            }
        };
        let report = command.finish();
        let status = report.status();
        let mut fields = vec![
            ("status", Value::string(status)),
            ("saved_paths", Value::number(report.saved)),
        ];
        if let Some(before) = report.before {
            fields.push(("before", Value::string(reference(before))));
        }
        if let Some(group) = report.group {
            fields.push(("group", Value::string(reference(group))));
        }
        if let Some(after) = report.after {
            fields.push(("after", Value::string(reference(after))));
        }
        let shown: Vec<Value> = report
            .changed
            .iter()
            .take(CHANGED_LIMIT)
            .map(Value::string)
            .collect();
        fields.push(("changed", Value::Array(shown)));
        if report.changed.len() > CHANGED_LIMIT {
            fields.push(("changed_count", Value::number(report.changed.len())));
        }
        if !report.partial.is_empty() {
            fields.push((
                "partial",
                Value::Array(
                    report
                        .partial
                        .iter()
                        .map(|(path, reason)| {
                            Value::object([
                                ("path", Value::string(path)),
                                ("reason", Value::string(reason)),
                            ])
                        })
                        .collect(),
                ),
            ));
        }
        if let Some(error) = report.error {
            fields.push((
                "error",
                Value::string(if report.before.is_some() {
                    format!("later changes may be incomplete: {error}")
                } else {
                    format!("the previous state was not saved: {error}")
                }),
            ));
        }
        let note = if running && status == "none" {
            "changed lists paths observed so far; their writer is unknown. The process is still running, and no previous state was saved"
        } else if running {
            "changed lists paths observed so far; their writer is unknown. The process is still running; its protected paths will be observed again when it exits"
        } else {
            "changed lists paths observed while this command ran; their writer is unknown"
        };
        fields.push(("note", Value::string(note)));
        object(fields)
    }
}

/// An object from fields chosen at run time.
pub(super) fn object(fields: Vec<(&str, Value)>) -> Value {
    Value::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
    )
}
