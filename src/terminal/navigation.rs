//! Validate a destination before releasing the current conversation owner.
use crate::{
    session::{self, persistence::Saved, scope::Directory},
    state::settings::Settings,
    workspace::{Access, Workspace},
};
use std::io;

pub(super) enum Request {
    New,
    Resume(String),
}
pub(super) struct Start {
    pub selected: Option<session::Model>,
    pub directory: Option<Directory>,
    pub workspace: Option<Workspace>,
    pub saved: Option<Saved>,
    pub prepared: Option<session::Session>,
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
            session::Session::with_directory(self.selected.unwrap(), directory.path(), workspace)?
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
                })
            }
            Request::Resume(id) => resume(&id, &self.directory),
        }
    }
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
    })
}
