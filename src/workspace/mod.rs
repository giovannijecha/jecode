//! Explicit workspace, opened once and addressed through native handles.
mod change;
mod diff;
mod directory;
mod path;
mod platform;
mod transaction;

pub use change::{Change, ChangeError, Preview};
pub(crate) use directory::Directory;
pub use path::relative;
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
pub use transaction::Applied;

pub const MAX_FILE_BYTES: usize = 1024 * 1024;
pub const MAX_DIRECTORY_ENTRIES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Path,
    Excluded,
    Unavailable,
    Text,
    Size,
    Changed,
    Cancelled,
    Timeout,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Path => {
                "use a relative workspace path with forward slashes and no parent traversal"
            }
            Self::Excluded => "path is excluded from workspace reads",
            Self::Unavailable => {
                "entry unavailable, unsupported or not an ordinary workspace file/directory"
            }
            Self::Text => "only UTF-8 text without binary control bytes is supported",
            Self::Size => "file exceeds the 1 MiB read limit",
            Self::Changed => "file changed while being read; no content returned",
            Self::Cancelled => "workspace operation cancelled",
            Self::Timeout => "workspace operation reached its time limit",
        })
    }
}
impl std::error::Error for Error {}

pub struct Budget<'a> {
    pub cancelled: &'a AtomicBool,
    pub deadline: Instant,
}
impl Budget<'_> {
    pub fn check(&self) -> Result<(), Error> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(Error::Cancelled)
        } else if Instant::now() >= self.deadline {
            Err(Error::Timeout)
        } else {
            Ok(())
        }
    }
}

pub struct Workspace {
    root: File,
    display: PathBuf,
}
pub struct Entry {
    pub name: String,
    pub directory: bool,
}
pub struct Listing {
    pub entries: Vec<Entry>,
    /// Omitted entries include excluded names, links and unsupported file kinds.
    pub omitted: usize,
    pub truncated: bool,
}
pub(super) struct Opened {
    file: File,
    // Windows ancestor handles deny rename/delete until the operation completes.
    _parents: Vec<File>,
}
impl Workspace {
    pub fn open(path: &Path) -> Result<Self, Error> {
        let display = path.canonicalize().map_err(|_| Error::Unavailable)?;
        let root = platform::root(&display).map_err(|_| Error::Unavailable)?;
        if !root.metadata().is_ok_and(|m| m.is_dir()) {
            return Err(Error::Unavailable);
        }
        Ok(Self { root, display })
    }
    pub fn path(&self) -> &Path {
        &self.display
    }
    pub fn list(&self, path: &str, budget: &Budget<'_>) -> Result<Listing, Error> {
        budget.check()?;
        let path = relative(path)?;
        let opened = platform::open(&self.root, &path, true).map_err(|_| Error::Unavailable)?;
        let mut names = Vec::new();
        let mut truncated = false;
        let mut omitted = 0;
        platform::names(&opened.file, &mut |name| {
            if budget.check().is_err() {
                return false;
            }
            if names.len() + omitted >= MAX_DIRECTORY_ENTRIES {
                truncated = true;
                return false;
            }
            match name.into_string() {
                Ok(name) if name != "." && name != ".." => names.push(name),
                Ok(_) => {}
                Err(_) => omitted += 1,
            }
            true
        })
        .map_err(|_| Error::Unavailable)?;
        budget.check()?;
        names.sort();
        let mut entries = Vec::new();
        for name in names {
            budget.check()?;
            let candidate = if path == "." {
                name.clone()
            } else {
                format!("{path}/{name}")
            };
            if relative(&candidate).is_err() {
                omitted += 1;
                continue;
            }
            // Resolve again through the root, never trust a directory entry's type.
            let directory = platform::open(&self.root, &candidate, true).is_ok();
            if directory || platform::open(&self.root, &candidate, false).is_ok() {
                entries.push(Entry { name, directory });
            } else {
                omitted += 1;
            }
        }
        Ok(Listing {
            entries,
            omitted,
            truncated,
        })
    }
    pub fn read(&self, path: &str, budget: &Budget<'_>) -> Result<String, Error> {
        budget.check()?;
        let path = relative(path)?;
        let mut opened =
            platform::open(&self.root, &path, false).map_err(|_| Error::Unavailable)?;
        let before = opened.file.metadata().map_err(|_| Error::Unavailable)?;
        if !before.is_file() {
            return Err(Error::Unavailable);
        }
        if before.len() > MAX_FILE_BYTES as u64 {
            return Err(Error::Size);
        }
        let mut bytes = Vec::new();
        let mut buffer = [0; 16 * 1024];
        loop {
            budget.check()?;
            let count = opened
                .file
                .read(&mut buffer)
                .map_err(|_| Error::Unavailable)?;
            if count == 0 {
                break;
            }
            if bytes.len() + count > MAX_FILE_BYTES {
                return Err(Error::Size);
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
        let after = opened.file.metadata().map_err(|_| Error::Unavailable)?;
        budget.check()?;
        if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
            return Err(Error::Changed);
        }
        let text = String::from_utf8(bytes).map_err(|_| Error::Text)?;
        if text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err(Error::Text);
        }
        Ok(text)
    }
}
