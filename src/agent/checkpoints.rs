//! The conversation's use of workspace checkpoints: its origin on every
//! conversation switch, recovery activity at each turn and notices about history
//! changed elsewhere, delivered before the next request.
use super::Agent;
use crate::checkpoints::Workspace;
use crate::checkpoints::command;
use crate::checkpoints::render::Output;
use crate::checkpoints::session::Session;
use crate::json::Value;
use std::path::Path;
use std::sync::Arc;

const NOTICE: &str = "checkpoint_notice";
const CURSOR: &str = "checkpoint_cursor";
/// Notices repeated in every request, so compaction and resume keep them.
const RECENT: usize = 5;

/// A local command detached from the agent so the terminal can run it off its
/// input thread without changing conversations while it is in progress.
pub(crate) struct CommandWork {
    workspace: Arc<Workspace>,
    origin: String,
    words: Vec<String>,
}

impl CommandWork {
    pub(crate) fn run(self) -> Result<Output, String> {
        command::execute(&self.workspace, &self.origin, &self.words, "/checkpoints")
    }
}

fn notice(event: &&Value) -> bool {
    event.get("type").and_then(Value::as_str) == Some(NOTICE)
}

fn cursor(event: &&Value) -> bool {
    matches!(
        event.get("type").and_then(Value::as_str),
        Some(NOTICE | CURSOR)
    )
}

impl Agent {
    /// Protects workspace changes with the archive under `home`. Opening
    /// never fails; an unavailable archive is reported by each protected call.
    pub fn enable_checkpoints(&mut self, home: &Path) {
        let session = Session::open(home, self.tools.root());
        self.use_checkpoints(session);
    }

    pub(crate) fn use_checkpoints(&mut self, session: Arc<Session>) {
        self.tools.configure_checkpoints(session);
        self.begin_checkpoints();
    }

    /// Runs `/checkpoints` words for the user of this conversation. The
    /// model learns about history changes through its next notice.
    pub fn checkpoints_command(&self, arguments: &str) -> Result<Output, String> {
        self.checkpoints_work(arguments)?.run()
    }

    pub(crate) fn checkpoints_work(&self, arguments: &str) -> Result<CommandWork, String> {
        let session = self
            .tools
            .checkpoints()
            .ok_or("Checkpoints are not enabled in this session")?;
        let words = command::split(arguments)?;
        let origin = format!("user:{}", session.origin());
        Ok(CommandWork {
            workspace: Arc::clone(session.workspace()?),
            origin,
            words,
        })
    }

    /// Starts notices and activity for the current conversation. A resumed
    /// conversation continues after the last notice it received.
    pub(super) fn begin_checkpoints(&self) {
        let Some(session) = self.tools.checkpoints() else {
            return;
        };
        let origin = self.persistence.as_ref().map_or_else(
            || format!("run-{}", crate::sessions::identifier()),
            crate::sessions::Handle::id,
        );
        let cursor = self
            .events
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(cursor)
            .and_then(|event| event.get("cursor").and_then(Value::as_usize))
            .map(|cursor| cursor as u64);
        session.begin(&origin, cursor);
    }

    /// Marks conversation activity without acquiring workspace contents.
    pub(super) fn checkpoint_turn(&self, label: &str) {
        if let Some(session) = self.tools.checkpoints() {
            let _ = session.turn(label);
        }
    }

    /// Tells other sessions this conversation is waiting for the user.
    pub(super) fn checkpoint_idle(&self) {
        if let Some(session) = self.tools.checkpoints() {
            session.activity("idle");
        }
    }

    /// Records history changes made outside this conversation's tool calls,
    /// reloading project instructions when a change touched JECODE.md.
    pub(super) fn deliver_checkpoint_notices(&mut self) {
        let Some(session) = self.tools.checkpoints().cloned() else {
            return;
        };
        let notice = session.notices();
        let current = session.cursor();
        let boundary = self.messages.lock().unwrap().len();
        {
            let mut events = self.events.lock().unwrap();
            let previous = events
                .iter()
                .rev()
                .find(cursor)
                .and_then(|event| event.get("cursor").and_then(Value::as_usize));
            // Even a read-only conversation must resume from the state its
            // requests observed, rather than a later turn's archive point.
            if previous.map(|value| value as u64) != Some(current) {
                events.push(Value::object([
                    ("type", Value::string(CURSOR)),
                    ("cursor", Value::number(current)),
                    ("after_message", Value::number(boundary)),
                ]));
            }
        }
        let Some(notice) = notice else {
            return;
        };
        let mut text = notice.text;
        if notice.instructions
            && let Err(error) = self.refresh_project_instructions()
        {
            text.push_str(&format!(
                "\nThe project instructions could not be reloaded: {error}"
            ));
        }
        self.events.lock().unwrap().push(Value::object([
            ("type", Value::string(NOTICE)),
            ("text", Value::string(self.redact(&text))),
            ("cursor", Value::number(notice.cursor)),
            ("after_message", Value::number(boundary)),
        ]));
        self.context.reset_usage();
    }

    /// The latest notices, for the request environment.
    pub(super) fn checkpoint_instructions(&self) -> String {
        let scope = self.tools.checkpoints()
            .and_then(|session| session.workspace().ok())
            .and_then(|workspace| workspace.selective())
            .map(|_| "Recovery is on demand. write/edit save each file's previous state automatically. For Bash that may change files, supply protect with file paths or recursive directories; `.` explicitly requests the whole workspace. Read the reported coverage: paths outside those scopes have no before-state guarantee. Use checkpoint.before to undo one step, or its group to undo the attempt. Inspect content/diffs only when useful. Accept your recovery group when satisfied with the result; tool success alone does not accept it. Accepted groups retain their initial states for at most 24 hours; active groups expire after 7 days of inactivity by default. User pins remain until released.".to_string());
        let events = self.events.lock().unwrap();
        let mut notices: Vec<&str> = events
            .iter()
            .rev()
            .filter(notice)
            .take(RECENT)
            .filter_map(|event| event.get("text").and_then(Value::as_str))
            .collect();
        if notices.is_empty() {
            return scope.unwrap_or_default();
        }
        notices.reverse();
        let notices = format!(
            "Checkpoint notices (workspace history changed outside your own tool calls; oldest first; use the checkpoint tool to inspect or undo):\n{}",
            notices.join("\n")
        );
        scope.map_or_else(|| notices.clone(), |scope| format!("{scope}\n\n{notices}"))
    }
}
