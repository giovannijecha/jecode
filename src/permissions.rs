//! Conversation permission levels (B12). The user chooses Read, Workspace or
//! Full; Jecode applies explicit tool boundaries. Workspace asks before file
//! access or a command outside the working folder, and remembered grants
//! extend Workspace for the rest of the conversation. On Windows, Workspace
//! commands also run inside the native sandbox (see `sandbox`).
//!
//! One shared state serves the main agent and its collaborators: they work
//! at the conversation's level, their requests reach the user one at a time
//! and grants apply to every agent. Level, grants and requests are journaled
//! in the conversation's session so resume restores them.

use crate::{cancel::Cancellation, export::Archive, json::Value, sessions::Handle};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub(crate) mod identity;
mod journal;
pub mod wording;
pub(crate) use journal::{pending_request, replay};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Level {
    Read,
    #[default]
    Workspace,
    Full,
}

impl Level {
    pub const ALL: [Self; 3] = [Self::Read, Self::Workspace, Self::Full];

    pub fn name(self) -> &'static str {
        match self {
            Self::Read => "Read",
            Self::Workspace => "Workspace",
            Self::Full => "Full",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Workspace => "workspace",
            Self::Full => "full",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.id() == id)
    }

    /// The short fact beside the level in `/permissions` and Settings.
    pub fn fact(self) -> &'static str {
        match self {
            Self::Read => "Read and analyze; no project changes",
            Self::Workspace => "Edit and run here and in subfolders; network on",
            Self::Full => "Read, write and run anywhere, as your user",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Reads,
    Edits,
    Runs,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Reads => "Reads",
            Self::Edits => "Edits",
            Self::Runs => "Runs",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Reads => "read",
            Self::Edits => "edit",
            Self::Runs => "run",
        }
    }

    fn parse(id: &str) -> Option<Self> {
        [Self::Reads, Self::Edits, Self::Runs]
            .into_iter()
            .find(|kind| kind.id() == id)
    }
}

/// What one call would touch. `target` is a path or a command; `place` is a
/// command's working directory when it is outside the folder.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Access {
    pub kind: Kind,
    pub target: String,
    pub place: Option<String>,
    pub outside: bool,
    /// The directory a file tool uses as its root for an outside path.
    pub base: Option<PathBuf>,
    /// Exact, private invocation identity for a remembered command grant.
    pub identity: Option<String>,
}

impl Access {
    pub fn grant(&self) -> Grant {
        Grant {
            kind: self.kind,
            target: self.target.clone(),
            place: self.place.clone(),
            identity: self.identity.clone(),
        }
    }
}

/// A remembered approval: the same access to the same target, never a
/// folder or command prefix.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Grant {
    pub kind: Kind,
    pub target: String,
    pub place: Option<String>,
    pub identity: Option<String>,
}

impl Grant {
    fn authorizes(&self, access: &Access) -> bool {
        self.kind == access.kind
            && self.place == access.place
            && match (&self.identity, &access.identity) {
                (Some(expected), Some(actual)) => expected == actual,
                (None, None) => self.target == access.target,
                _ => false,
            }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decision {
    Once,
    Remember,
    Deny,
}

impl Decision {
    fn id(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Remember => "remember",
            Self::Deny => "deny",
        }
    }
}

pub enum Verdict {
    Allow,
    Unavailable(String),
    Ask,
}

pub enum Asked {
    Decided(Decision),
    Cancelled,
}

/// Where requests reach the user. The TUI polls the queue for every agent;
/// the line-based CLI asks inline for the main agent only.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Surface {
    #[default]
    None,
    Inline,
    Panel,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Request {
    pub id: u64,
    pub call_id: String,
    /// The collaborator's ID (`a1`); `None` for the main agent.
    pub agent: Option<String>,
    pub tool: String,
    pub access: Access,
}

#[derive(Default)]
struct State {
    level: Level,
    default: Level,
    grants: Vec<Grant>,
    queue: VecDeque<Request>,
    decisions: BTreeMap<u64, Decision>,
    next: u64,
    surface: Surface,
    archive: Option<Archive>,
    handle: Option<Handle>,
}

impl State {
    fn journal(&self, event: Value) -> Result<(), String> {
        match (&self.handle, &self.archive) {
            (Some(handle), Some(archive)) => handle.append_event(archive, event).map(|_| ()),
            _ => Ok(()),
        }
    }
}

/// The conversation's permissions, shared by every agent; each clone names
/// the agent that asks through it.
#[derive(Clone, Default)]
pub struct Permissions {
    shared: Arc<(Mutex<State>, Condvar)>,
    agent: Option<String>,
    journal: Arc<Mutex<Option<(Archive, Handle)>>>,
}

impl Permissions {
    pub fn for_agent(&self, id: &str) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            agent: Some(id.into()),
            journal: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn bind_agent_journal(&self, archive: Archive, handle: Option<Handle>) {
        *self.journal.lock().unwrap() = handle.map(|handle| (archive, handle));
    }

    fn journal_agent(&self, event: Value) -> Result<(), String> {
        if let Some((archive, handle)) = &*self.journal.lock().unwrap() {
            handle.append_event(archive, event).map(|_| ())
        } else {
            Ok(())
        }
    }

    /// Mark a collaborator's answered request before its tool can run.
    pub(crate) fn record_agent_decision(
        &self,
        request: &Request,
        decision: Decision,
    ) -> Result<(), String> {
        if self.agent.is_some() {
            self.journal_agent(journal::decision(request, decision))?;
        }
        Ok(())
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.shared.0.lock().unwrap()
    }

    pub fn level(&self) -> Level {
        self.state().level
    }

    /// The saved default for new conversations; this one keeps its level.
    pub fn default_level(&self) -> Level {
        self.state().default
    }

    pub fn set_default(&self, level: Level) {
        self.state().default = level;
    }

    /// Applies from the next call; calls already running keep theirs.
    pub fn set_level(&self, level: Level) -> Result<(), String> {
        let mut state = self.state();
        if state.level == level {
            return Ok(());
        }
        state.journal(journal::level(level))?;
        state.level = level;
        Ok(())
    }

    pub fn grants(&self) -> Vec<Grant> {
        self.state().grants.clone()
    }

    pub fn revoke(&self, grant: &Grant) -> Result<(), String> {
        let mut state = self.state();
        let index = state
            .grants
            .iter()
            .position(|known| known == grant)
            .ok_or("This grant is no longer remembered")?;
        state.journal(journal::grant("permission_revoke", grant))?;
        state.grants.remove(index);
        Ok(())
    }

    pub fn check(&self, access: &Access) -> Verdict {
        self.assess(access).1
    }

    /// The decision and level come from the same state snapshot.
    pub fn assess(&self, access: &Access) -> (Level, Verdict) {
        let state = self.state();
        let verdict = match state.level {
            Level::Full => Verdict::Allow,
            Level::Read if access.kind == Kind::Reads && !access.outside => Verdict::Allow,
            Level::Read => Verdict::Unavailable(format!(
                "Not available at the Read permission level: {}. Read allows reading and analysis inside the working folder; explain what you need and the user can change the level with /permissions.",
                describe(access)
            )),
            Level::Workspace if !access.outside => Verdict::Allow,
            Level::Workspace if state.grants.iter().any(|grant| grant.authorizes(access)) => {
                Verdict::Allow
            }
            Level::Workspace => Verdict::Ask,
        };
        (state.level, verdict)
    }

    pub fn set_surface(&self, surface: Surface) {
        self.state().surface = surface;
        self.shared.1.notify_all();
    }

    /// The request shown now and how many wait in all.
    pub fn pending(&self) -> Option<(Request, usize)> {
        let state = self.state();
        state
            .queue
            .front()
            .map(|request| (request.clone(), state.queue.len()))
    }

    pub fn decide(&self, id: u64, decision: Decision) -> Result<(), String> {
        let mut state = self.state();
        let index = state
            .queue
            .iter()
            .position(|request| request.id == id)
            .ok_or("This request is no longer waiting")?;
        let request = state.queue[index].clone();
        // Child call IDs may equal main call IDs. Its own session records the
        // answer; the parent records only a grant when one is remembered.
        // A failed append leaves the queue and grant state unchanged.
        if request.agent.is_some() {
            if decision == Decision::Remember {
                state.journal(journal::grant("permission_grant", &request.access.grant()))?;
            }
        } else {
            state.journal(journal::decision(&request, decision))?;
        }
        if decision == Decision::Remember {
            let grant = request.access.grant();
            if !state.grants.contains(&grant) {
                state.grants.push(grant);
            }
        }
        state.queue.remove(index);
        state.decisions.insert(id, decision);
        self.shared.1.notify_all();
        Ok(())
    }

    /// Queues a request; `None` when no surface can show it.
    pub fn begin(&self, call_id: &str, tool: &str, access: &Access) -> Option<Request> {
        let request = {
            let mut state = self.state();
            match state.surface {
                Surface::None => return None,
                Surface::Inline if self.agent.is_some() => return None,
                Surface::Inline | Surface::Panel => {}
            }
            state.next += 1;
            let request = Request {
                id: state.next,
                call_id: call_id.into(),
                agent: self.agent.clone(),
                tool: tool.into(),
                access: access.clone(),
            };
            // Each request belongs to the calling agent's own session.
            let saved = if self.agent.is_some() {
                self.journal_agent(journal::request(&request))
            } else {
                state.journal(journal::request(&request))
            };
            if saved.is_err() {
                return None;
            }
            state.queue.push_back(request.clone());
            request
        };
        self.shared.1.notify_all();
        Some(request)
    }

    /// Waits, unrun, until the user decides; only the asking agent waits.
    pub fn wait(&self, id: u64, cancellation: &Cancellation) -> Asked {
        let mut state = self.state();
        loop {
            if let Some(decision) = state.decisions.remove(&id) {
                return Asked::Decided(decision);
            }
            if cancellation.requested() || state.surface == Surface::None {
                state.queue.retain(|request| request.id != id);
                self.shared.1.notify_all();
                return Asked::Cancelled;
            }
            state = self
                .shared
                .1
                .wait_timeout(state, Duration::from_millis(100))
                .unwrap()
                .0;
        }
    }

    /// Withdraws an unshown request when its event cannot reach the UI.
    pub(crate) fn abandon(&self, id: u64) {
        let mut state = self.state();
        state.queue.retain(|request| request.id != id);
        state.decisions.remove(&id);
        self.shared.1.notify_all();
    }

    /// Restores the level and grants of the bound conversation: the last
    /// level change, else the level it started at, else the default.
    pub(crate) fn bind(&self, archive: Archive, handle: Option<Handle>) {
        let mut state = self.state();
        state.queue.clear();
        state.decisions.clear();
        state.grants.clear();
        state.level = state.default;
        if let Some(handle) = &handle {
            let document = handle.snapshot();
            let (level, grants) = replay(&document.events);
            state.level = level.or(document.permissions).unwrap_or(state.default);
            state.grants = grants;
        }
        state.archive = Some(archive);
        state.handle = handle;
        self.shared.1.notify_all();
    }
}

/// `read ~\a.rs outside the working folder`, for the model.
pub fn describe(access: &Access) -> String {
    let verb = match access.kind {
        Kind::Reads => "reading",
        Kind::Edits => "editing",
        Kind::Runs => "running",
    };
    let mut text = format!("{verb} {}", access.target);
    if let Some(place) = &access.place {
        text.push_str(&format!(" in {place}"));
    }
    if access.outside {
        text.push_str(" outside the working folder");
    }
    text
}

/// A path as people write it, without Windows' verbatim prefix.
pub fn plain(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
}

#[cfg(test)]
mod tests;
