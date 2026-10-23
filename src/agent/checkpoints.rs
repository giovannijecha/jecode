//! Conversation access to per-call workspace recovery.
use super::Agent;
use crate::checkpoints::Store;
use crate::checkpoints::command;
use crate::checkpoints::render::Output;
use crate::checkpoints::session::Session;
use std::path::Path;
use std::sync::Arc;

/// A local command detached from the terminal input thread.
pub(crate) struct CommandWork {
    store: Arc<Store>,
    words: Vec<String>,
}

impl CommandWork {
    pub(crate) fn run(self) -> Result<Output, String> {
        command::run(&self.store, &self.words)
    }
}

impl Agent {
    pub fn enable_checkpoints(&mut self, home: &Path) {
        self.use_checkpoints(Session::open(home, self.tools.root()));
    }

    pub(crate) fn use_checkpoints(&mut self, session: Arc<Session>) {
        self.tools.configure_checkpoints(session);
        self.begin_checkpoints();
    }

    pub fn checkpoints_command(&self, arguments: &str) -> Result<Output, String> {
        self.checkpoints_work(arguments)?.run()
    }

    pub(crate) fn checkpoints_work(&self, arguments: &str) -> Result<CommandWork, String> {
        let session = self
            .tools
            .checkpoints()
            .ok_or("Checkpoints are not enabled in this session")?;
        Ok(CommandWork {
            store: Arc::clone(session.store()?),
            words: command::split(arguments)?,
        })
    }

    pub(super) fn begin_checkpoints(&self) {
        let Some(session) = self.tools.checkpoints() else {
            return;
        };
        let origin = self.persistence.as_ref().map_or_else(
            || format!("run-{}", crate::sessions::identifier()),
            crate::sessions::Handle::id,
        );
        session.begin(&origin);
    }

    /// Keep project instructions fresh after this conversation restores
    /// JECODE.md. Recovery has no cross-session event feed.
    pub(super) fn reload_restored_instructions(&mut self) {
        if self
            .tools
            .checkpoints()
            .is_some_and(|session| session.take_restored_instructions())
        {
            let _ = self.refresh_project_instructions();
            self.context.reset_usage();
        }
    }

    pub(super) fn checkpoint_instructions(&self) -> String {
        if self.tools.checkpoints().is_none() {
            return String::new();
        }
        "Recovery is per modifying call. write/edit save each file's previous state automatically; for Bash, declare protect paths or recursive directories before launch. Each saved ref expires 24 hours after capture. Use checkpoint list/inspect/restore; inspect supplies the expected live-state token for restore, or pass latest to accept the current state. Read checkpoint:rc-ID:path for a saved file. A receipt describes captured preimages, not whether a command succeeded.".into()
    }
}
