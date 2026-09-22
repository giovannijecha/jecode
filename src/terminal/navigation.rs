//! Validate a destination before releasing the current conversation owner.
use crate::{
    session::{self, persistence::Saved},
    state::settings::Settings,
    workspace::{Access, Workspace},
};
use std::{io, path::PathBuf};

pub(super) enum Request {
    New,
    Resume(String),
}
pub(super) struct Start {
    pub selected: Option<session::Model>,
    pub workspace: Option<Workspace>,
    pub saved: Option<Saved>,
}
pub(super) struct Location {
    pub path: Option<PathBuf>,
    pub access: Access,
}
impl Location {
    pub fn from_workspace(workspace: Option<&Workspace>) -> Self {
        Self {
            path: workspace.map(|w| w.path().to_owned()),
            access: workspace.map_or(Access::Workspace, Workspace::access),
        }
    }
    pub fn resolve(&self, request: Request) -> io::Result<Start> {
        match request {
            Request::New => {
                let settings = Settings::user()?;
                let workspace = self
                    .path
                    .as_ref()
                    .map(|path| {
                        Workspace::open(path)
                            .map(|w| w.with_access(settings.file_access))
                            .map_err(|_| io::Error::other("working directory unavailable"))
                    })
                    .transpose()?;
                Ok(Start {
                    selected: Some(settings.model),
                    workspace,
                    saved: None,
                })
            }
            Request::Resume(id) => resume(&id),
        }
    }
}
pub(super) fn resume(id: &str) -> io::Result<Start> {
    let saved = session::persistence::resume(id)?;
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
        workspace,
        saved: Some(saved),
    })
}
