//! Directory association for discovery, independent of file-tool access.
use crate::workspace::Workspace;
use std::{
    io,
    path::{Path, PathBuf},
};

pub struct Directory {
    path: PathBuf,
    identity: (u64, u64),
}
impl Directory {
    pub fn open(path: &Path) -> io::Result<Self> {
        let workspace = Workspace::open(path)
            .map_err(|_| io::Error::other("selected working directory is unavailable"))?;
        Ok(Self {
            path: workspace.path().to_owned(),
            identity: workspace
                .identity()
                .map_err(|_| io::Error::other("working directory identity is unavailable"))?,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn contains(&self, saved: &Path) -> bool {
        Workspace::open(saved)
            .ok()
            .and_then(|workspace| workspace.identity().ok())
            == Some(self.identity)
    }
    pub fn require(&self, saved: Option<&Path>) -> io::Result<()> {
        match saved {
            Some(path) => {
                let workspace = Workspace::open(path).map_err(|_| io::Error::new(io::ErrorKind::NotFound,
                    format!("saved directory unavailable: {path:?}; restore that directory to resume this session")))?;
                if workspace.identity().ok() == Some(self.identity) {
                    Ok(())
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!(
                            "session belongs to another directory: {path:?}; run jecode --workspace PATH resume SESSION_ID with PATH set to that directory"
                        ),
                    ))
                }
            }
            None => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "legacy session has no saved directory; its origin cannot be inferred and its file is preserved",
            )),
        }
    }
}
