//! User-owned JSON and session files. Historical ~/.jecode files are never imported.
mod platform;
pub mod settings;
#[cfg(test)]
pub(crate) mod tests;

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Store {
    root: PathBuf,
}

/// One logical owner, even while a forked child temporarily inherits the file.
#[derive(Debug)]
pub struct Lease {
    file: File,
}
impl Drop for Lease {
    fn drop(&mut self) {
        // Closing just this descriptor can leave a Linux flock held by a child
        // before exec. Explicitly release ownership before closing our handle.
        let _ = self.file.unlock();
    }
}

impl Store {
    pub fn user() -> io::Result<Self> {
        let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let home = std::env::var_os(key)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| io::Error::other("user home directory is unavailable"))?;
        Self::in_home(&home)
    }

    /// Explicit roots let tests use isolated fixtures without reading real credentials.
    pub fn in_home(home: &Path) -> io::Result<Self> {
        if !home.is_absolute() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let base = home.join(".jecode");
        directory(&base, false)?;
        Self::open(base.join("v1"))
    }

    fn open(root: PathBuf) -> io::Result<Self> {
        directory(&root, true)?;
        Ok(Self { root })
    }

    pub fn directory(&self, name: &str) -> io::Result<Self> {
        Self::open(self.path(name)?)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub(crate) fn parent(&self) -> io::Result<Self> {
        Self::open(
            self.root
                .parent()
                .ok_or(io::ErrorKind::InvalidInput)?
                .to_owned(),
        )
    }

    pub fn names(&self) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for (index, entry) in fs::read_dir(&self.root)?.enumerate() {
            if index >= 10_000 {
                return Err(io::Error::other("too many local state files"));
            }
            let entry = entry?;
            if entry.file_type()?.is_file()
                && let Some(name) = entry.file_name().to_str()
                && valid_name(name)
            {
                names.push(name.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }

    pub fn read(&self, name: &str, limit: usize) -> io::Result<Option<String>> {
        let path = self.path(name)?;
        let file = match platform::read(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        platform::check_file(&file)?;
        if file.metadata()?.len() > limit as u64 {
            return Err(io::Error::other("local state file exceeds its size limit"));
        }
        let mut text = String::new();
        file.take(limit as u64 + 1).read_to_string(&mut text)?;
        if text.len() > limit {
            return Err(io::Error::other("local state file exceeds its size limit"));
        }
        Ok(Some(text))
    }

    /// Open a private owned data file for bounded incremental I/O. Callers
    /// hold a session lease; this does not establish transaction boundaries.
    pub(crate) fn data_file(&self, name: &str, existing: bool) -> io::Result<File> {
        let file = platform::create(&self.path(name)?, existing)?;
        platform::check_file(&file)?;
        Ok(file)
    }

    pub(crate) fn read_file(&self, name: &str) -> io::Result<File> {
        let file = platform::read(&self.path(name)?)?;
        platform::check_file(&file)?;
        Ok(file)
    }

    pub(crate) fn sync_root(&self) -> io::Result<()> {
        platform::sync_directory(&self.root)
    }

    /// Callers hold the corresponding lock across load/modify/replace sequences.
    pub fn replace(&self, name: &str, contents: &str) -> io::Result<()> {
        self.replace_bytes(name, contents.as_bytes())
    }

    /// Atomically commit private binary evidence under the same protected root.
    pub(crate) fn replace_bytes(&self, name: &str, contents: &[u8]) -> io::Result<()> {
        let destination = self.path(name)?;
        if destination.symlink_metadata().is_ok() {
            platform::check_file(&platform::read(&destination)?)?;
        }
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let mut attempts = 0;
        let (temporary, mut file) = loop {
            attempts += 1;
            if attempts > 100 {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            let temporary = self.path(&format!(
                "pending-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))?;
            match platform::create(&temporary, false) {
                Ok(file) => break (temporary, file),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        };
        let result = (|| {
            file.write_all(contents)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &destination)?;
            platform::sync_directory(&self.root)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    /// Keep the lock file to preserve its identity; the lease releases ownership.
    pub fn lock(&self, name: &str, cancelled: &AtomicBool, deadline: Instant) -> io::Result<Lease> {
        let file = platform::create(&self.path(name)?, true)?;
        platform::check_file(&file)?;
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            match file.try_lock() {
                Ok(()) => return Ok(Lease { file }),
                Err(fs::TryLockError::Error(error)) => return Err(error),
                Err(fs::TryLockError::WouldBlock) => {}
            }
            if Instant::now() >= deadline {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn path(&self, name: &str) -> io::Result<PathBuf> {
        if !valid_name(name) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        if !ordinary_directory(&self.root)? {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        Ok(self.root.join(name))
    }
}

fn valid_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    !name.is_empty()
        && name.len() <= 128
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && !name.ends_with('.')
        && !matches!(
            stem.as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        )
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

fn directory(path: &Path, private: bool) -> io::Result<()> {
    match platform::directory(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    if !ordinary_directory(path)? {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    if private {
        platform::protect_directory(path)?;
    }
    Ok(())
}
fn ordinary_directory(path: &Path) -> io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Ok(false);
        }
    }
    Ok(metadata.is_dir() && !metadata.file_type().is_symlink())
}
