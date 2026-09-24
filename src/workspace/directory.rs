//! A held, validated starting directory. This does not sandbox a shell's effects.
use super::{Access, Budget, Error, Opened, Workspace, platform};
use std::{fs::File, path::PathBuf};
#[cfg(windows)]
use std::{
    io,
    os::windows::ffi::OsStrExt,
    path::{Component, Prefix},
};

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

    #[cfg(windows)]
    pub fn process_path(&self) -> io::Result<PathBuf> {
        let unsupported = || {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "unsupported Windows shell starting directory",
            )
        };
        let mut parts = self.path.components();
        let drive = match parts.next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
                _ => return Err(unsupported()),
            },
            _ => return Err(unsupported()),
        };
        if parts.next() != Some(Component::RootDir) {
            return Err(unsupported());
        }
        let mut ordinary = PathBuf::from(format!("{}:\\", char::from(drive)));
        for part in parts {
            match part {
                Component::Normal(name) => ordinary.push(name),
                _ => return Err(unsupported()),
            }
        }
        // Windows PowerShell's provider does not resolve relative paths from a
        // verbatim cwd. CreateProcessW cannot use a cwd beyond MAX_PATH either.
        // Leave long directory access to file tools; never run in another cwd.
        if ordinary.as_os_str().encode_wide().count() > 258 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Windows PowerShell cannot use a starting directory longer than MAX_PATH",
            ));
        }
        let reopened = platform::root(&ordinary).map_err(|_| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "Windows PowerShell cannot use the selected starting directory",
            )
        })?;
        if platform::identity(&reopened)? != self.identity {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Windows PowerShell starting directory does not match the selected directory",
            ));
        }
        Ok(ordinary)
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
