use super::{Area, MARKER, inspect};
use crate::json::{self, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

/// A fully inspected, owned session area ready for one deletion attempt.
pub(crate) struct Removal {
    root: PathBuf,
    identity: Value,
    entries: Vec<(PathBuf, bool)>,
    files: usize,
    directories: usize,
}

impl Removal {
    pub(crate) fn remove(self) -> Result<(usize, usize), String> {
        for (removed, (path, directory)) in self.entries.iter().enumerate() {
            if path != &self.root {
                let owner = fs::read_to_string(self.root.join(MARKER)).map_err(|error| {
                    format!("Temporary deletion is incomplete after {removed} entries: ownership record changed: {error}")
                })?;
                if json::parse(&owner).ok() != Some(self.identity.clone()) {
                    return Err(format!(
                        "Temporary deletion is incomplete after {removed} entries: ownership record changed"
                    ));
                }
            }
            // Check every ancestor we own before using a prepared path. This also
            // catches a swapped junction between preflight and removal.
            let mut ancestors = path
                .ancestors()
                .take_while(|parent| *parent != self.root.parent().unwrap())
                .collect::<Vec<_>>();
            ancestors.reverse();
            for parent in ancestors {
                let metadata = inspect(parent).map_err(|error| {
                    format!("Temporary deletion is incomplete after {removed} entries: {error}")
                })?;
                if parent != path && !metadata.is_dir() {
                    return Err(format!(
                        "Temporary deletion is incomplete after {removed} entries: a parent changed"
                    ));
                }
            }
            let metadata = inspect(path).map_err(|error| {
                format!("Temporary deletion is incomplete after {removed} entries: {error}")
            })?;
            if metadata.is_dir() != *directory || !(metadata.is_dir() || metadata.is_file()) {
                return Err(format!(
                    "Temporary deletion is incomplete after {removed} entries: an entry changed"
                ));
            }
            let result = if *directory {
                fs::remove_dir(path)
            } else {
                fs::remove_file(path)
            };
            if let Err(error) = result {
                let failure =
                    format!("Temporary deletion is incomplete after {removed} entries: {error}");
                if path == &self.root {
                    return match self.restore_marker() {
                        Ok(()) => Err(format!(
                            "{failure}. Temporary ownership was restored for a retry"
                        )),
                        Err(restore_error) => Err(format!(
                            "{failure}. Could not restore temporary ownership: {restore_error}"
                        )),
                    };
                }
                return Err(failure);
            }
        }
        Ok((self.files, self.directories))
    }

    fn restore_marker(&self) -> Result<(), String> {
        for parent in self
            .root
            .ancestors()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            if !inspect(parent)?.is_dir() {
                return Err("Temporary area parent changed after cleanup failure".into());
            }
        }
        let marker = self.root.join(MARKER);
        match fs::symlink_metadata(&marker) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err("Temporary ownership path was replaced after cleanup failure".into());
            }
            Err(error) => return Err(error.to_string()),
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&marker)
            .map_err(|error| format!("Could not recreate temporary ownership: {error}"))?;
        file.write_all(self.identity.encode().as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("Could not save temporary ownership: {error}"))?;
        drop(file);
        if !inspect(&marker)?.is_file()
            || json::parse(&fs::read_to_string(&marker).map_err(|error| error.to_string())?)?
                != self.identity
        {
            return Err("Restored temporary ownership changed".into());
        }
        Ok(())
    }
}

impl Area {
    /// Inspect an existing area without creating a missing one. The marker and
    /// root are included in the removal count and ordered after their contents.
    pub(crate) fn prepare_removal(&self) -> Result<Option<Removal>, String> {
        for parent in self
            .path
            .ancestors()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            match fs::symlink_metadata(parent) {
                Ok(_) if !inspect(parent)?.is_dir() => {
                    return Err("Temporary area parent must be a directory".into());
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(format!("Could not inspect temporary area: {error}")),
            }
        }
        let marker = self.path.join(MARKER);
        if !inspect(&marker)?.is_file() {
            return Err("Temporary ownership record must be a regular file".into());
        }
        let owner = fs::read_to_string(&marker)
            .map_err(|error| format!("Could not read temporary area ownership: {error}"))?;
        if json::parse(&owner)? != self.identity() {
            return Err(
                "Temporary area belongs to another folder or session; files were left unchanged"
                    .into(),
            );
        }
        let mut entries = Vec::new();
        let mut pending = vec![(self.path.clone(), false)];
        let mut files = 0;
        let mut directories = 0;
        while let Some((path, visited)) = pending.pop() {
            let metadata = inspect(&path)?;
            if metadata.is_file() {
                files += 1;
                entries.push((path, false));
            } else if metadata.is_dir() {
                if visited {
                    if path == self.path {
                        files += 1;
                        entries.push((marker.clone(), false));
                    }
                    directories += 1;
                    entries.push((path, true));
                } else {
                    pending.push((path.clone(), true));
                    for entry in fs::read_dir(&path)
                        .map_err(|error| format!("Could not inspect temporary area: {error}"))?
                    {
                        let entry = entry.map_err(|error| error.to_string())?.path();
                        if entry != marker {
                            pending.push((entry, false));
                        }
                    }
                }
            } else {
                return Err(
                    "Temporary area contains a special file; files were left unchanged".into(),
                );
            }
        }
        Ok(Some(Removal {
            root: self.path.clone(),
            identity: self.identity(),
            entries,
            files,
            directories,
        }))
    }
}
