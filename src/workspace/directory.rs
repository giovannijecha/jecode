//! A held, validated starting directory. This does not sandbox a shell's effects.
use super::{Access, Budget, Error, Opened, Workspace, platform};
use std::{fs::File, path::PathBuf};

pub(crate) struct Directory {
    opened: Opened,
    pub path: PathBuf,
    request_path: String,
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
        let location = self.resolve(path)?;
        let relative = location.display.clone();
        let opened = self.open_location(&location, true)?;
        let identity = platform::identity(&opened.file).map_err(|_| Error::Unavailable)?;
        Ok(Directory {
            opened,
            path: if self.access == Access::Local {
                PathBuf::from(&relative)
            } else {
                self.display.join(&relative)
            },
            request_path: relative,
            identity,
        })
    }
    pub(crate) fn validate_directory(
        &self,
        directory: &Directory,
        budget: &Budget<'_>,
    ) -> Result<(), Error> {
        let current = self.directory(&directory.request_path, budget)?;
        if current.identity != directory.identity {
            return Err(Error::Changed);
        }
        Ok(())
    }
}
