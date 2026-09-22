//! A held, validated starting directory. This does not sandbox a shell's effects.
use super::{Budget, Error, Opened, Workspace, platform, relative};
use std::{fs::File, path::PathBuf};

pub(crate) struct Directory {
    opened: Opened,
    pub path: PathBuf,
    relative: String,
    identity: (u64, u64),
}
impl Directory {
    pub fn file(&self) -> &File {
        &self.opened.file
    }
}
impl Workspace {
    pub(crate) fn directory(&self, path: &str, budget: &Budget<'_>) -> Result<Directory, Error> {
        budget.check()?;
        let relative = relative(path)?;
        let opened = platform::open(&self.root, &relative, true).map_err(|_| Error::Unavailable)?;
        let identity = platform::identity(&opened.file).map_err(|_| Error::Unavailable)?;
        Ok(Directory {
            opened,
            path: self.display.join(&relative),
            relative,
            identity,
        })
    }
    pub(crate) fn validate_directory(
        &self,
        directory: &Directory,
        budget: &Budget<'_>,
    ) -> Result<(), Error> {
        let current = self.directory(&directory.relative, budget)?;
        if current.identity != directory.identity {
            return Err(Error::Changed);
        }
        Ok(())
    }
}
