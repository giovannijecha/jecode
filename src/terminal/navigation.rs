//! Validate a destination before releasing the current conversation owner.
use crate::{
    session::{self, persistence::Saved, scope::Directory},
    state::settings::Settings,
    workspace::{Access, Workspace},
};
use std::io;

pub(super) enum Request {
    New,
    Clear,
    Resume(String),
}
pub(super) struct Carried {
    pub blocks: Vec<super::model::Block>,
    pub tools: std::collections::BTreeMap<usize, super::lab::model::Tool>,
    pub receipts: std::collections::BTreeMap<usize, super::lab::model::Receipt>,
    pub previous_id: Option<String>,
}
pub(super) struct Pending {
    pub editor: super::editor::Editor,
    pub queued: std::collections::VecDeque<super::account::Queued>,
    pub pasted_literal: bool,
}
impl Pending {
    pub fn capture(model: &super::model::Model) -> Option<Self> {
        model.account.as_ref().map(|view| Self {
            editor: model.editor.clone(),
            queued: view.queued_turns.clone(),
            pasted_literal: model.menu.pasted_literal,
        })
    }
    pub fn restore(self, model: &mut super::model::Model) {
        model.editor = self.editor;
        model.menu.pasted_literal = self.pasted_literal;
        if let Some(view) = &mut model.account {
            view.queued_turns = self.queued;
        }
    }
}
pub(super) struct Start {
    pub selected: Option<session::Model>,
    pub directory: Option<Directory>,
    pub workspace: Option<Workspace>,
    pub saved: Option<Saved>,
    pub prepared: Option<session::Session>,
    pub carried: Option<Carried>,
    pub pending: Option<Pending>,
}
impl Start {
    /// Complete the fallible resume while the previous conversation still owns its draft.
    pub fn prepare(&mut self) -> io::Result<()> {
        let Some(directory) = &self.directory else {
            return Ok(());
        };
        let workspace = self
            .workspace
            .as_ref()
            .map(|workspace| {
                Workspace::open(workspace.path())
                    .map(|opened| opened.with_access(workspace.access()))
                    .map_err(|_| io::Error::other("saved working directory unavailable"))
            })
            .transpose()?;
        let session = if let Some(saved) = self.saved.take() {
            session::Session::resume(saved, directory, workspace)?
        } else {
            session::Session::with_directory_from(
                self.selected.unwrap(),
                directory.path(),
                workspace,
                self.carried
                    .as_ref()
                    .and_then(|carried| carried.previous_id.as_deref()),
            )?
        };
        self.prepared = Some(session);
        Ok(())
    }
}
pub(super) struct Location {
    pub directory: Directory,
    pub file_tools: bool,
    pub access: Access,
}
impl Location {
    pub fn new(directory: Directory, workspace: Option<&Workspace>) -> Self {
        Self {
            directory,
            file_tools: workspace.is_some(),
            access: workspace.map_or(Access::Workspace, Workspace::access),
        }
    }
    pub fn resolve(&self, request: Request) -> io::Result<Start> {
        match request {
            Request::New => {
                let settings = Settings::user()?;
                let directory = Directory::open(self.directory.path())?;
                let workspace = self
                    .file_tools
                    .then(|| {
                        Workspace::open(directory.path())
                            .map(|w| w.with_access(settings.file_access))
                            .map_err(|_| io::Error::other("working directory unavailable"))
                    })
                    .transpose()?;
                Ok(Start {
                    selected: Some(settings.model),
                    directory: Some(directory),
                    workspace,
                    saved: None,
                    prepared: None,
                    carried: None,
                    pending: None,
                })
            }
            Request::Clear => {
                let directory = Directory::open(self.directory.path())?;
                let workspace = self
                    .file_tools
                    .then(|| {
                        Workspace::open(directory.path())
                            .map(|opened| opened.with_access(self.access))
                            .map_err(|_| io::Error::other("working directory unavailable"))
                    })
                    .transpose()?;
                Ok(Start {
                    selected: None,
                    directory: Some(directory),
                    workspace,
                    saved: None,
                    prepared: None,
                    carried: None,
                    pending: None,
                })
            }
            Request::Resume(id) => resume(&id, &self.directory),
        }
    }
}
/// Validate and prepare before handing pending work to the next controller.
pub(super) fn resolve(model: &mut super::model::Model, location: &Location) -> Option<Start> {
    let request = model.navigation.take()?;
    let clear = matches!(request, Request::Clear);
    let mut next = match location.resolve(request) {
        Ok(next) => next,
        Err(_) => {
            if model
                .account
                .as_ref()
                .is_some_and(|view| !view.queued_turns.is_empty())
            {
                super::account::recover_queued(model);
            }
            if let Some(view) = &mut model.account {
                view.local_notice = "Cannot open that conversation · check its folder or another running owner · current session kept".into();
                view.local_failed = false;
            }
            return None;
        }
    };
    if clear {
        next.selected = model.account.as_ref().map(|view| view.selected);
        next.carried = Some(Carried {
            blocks: model.blocks.clone(),
            tools: model.tool_details.clone(),
            receipts: model.command_receipts.clone(),
            previous_id: model.account.as_ref().and_then(|view| view.id.clone()),
        });
    }
    if next.prepare().is_err() {
        if model
            .account
            .as_ref()
            .is_some_and(|view| !view.queued_turns.is_empty())
        {
            super::account::recover_queued(model);
        }
        if let Some(view) = &mut model.account {
            view.local_notice = "Cannot open that conversation · check its directory or another owner · current session and draft kept".into();
            view.local_failed = false;
        }
        return None;
    }
    next.pending = Pending::capture(model);
    Some(next)
}
pub(super) fn resume(id: &str, directory: &Directory) -> io::Result<Start> {
    let saved = session::persistence::resume_in(id, directory)?;
    let workspace = saved
        .workspace
        .as_ref()
        .map(|path| {
            Workspace::open(path)
                .map(|w| w.with_access(saved.access))
                .map_err(|_| io::Error::other("saved workspace unavailable"))
        })
        .transpose()?;
    Ok(Start {
        selected: Some(saved.model),
        directory: Some(Directory::open(directory.path())?),
        workspace,
        saved: Some(saved),
        prepared: None,
        carried: None,
        pending: None,
    })
}
