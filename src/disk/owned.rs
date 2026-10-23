//! JSON ownership records and removal of session-owned directories.
use crate::json::{self, Value};
use std::fs::{self, Metadata};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub struct Marker {
    pub name: &'static str,
    pub format: &'static str,
    pub label: &'static str,
    pub max_bytes: Option<u64>,
}

impl Marker {
    pub fn identity(self, project: &Path, session: &str) -> Value {
        Value::object([
            ("format", Value::string(self.format)),
            ("format_version", Value::number(1)),
            ("directory", Value::string(project.to_string_lossy())),
            ("session_id", Value::string(session)),
        ])
    }

    pub fn read(self, directory: &Path) -> Result<Value, String> {
        let path = directory.join(self.name);
        let metadata = self.inspect(&path)?;
        if !metadata.is_file() || self.max_bytes.is_some_and(|limit| metadata.len() > limit) {
            return Err(format!(
                "{} ownership record must be a regular{} file",
                self.label,
                if self.max_bytes.is_some() {
                    " small"
                } else {
                    ""
                }
            ));
        }
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("Could not read {} ownership: {error}", self.label))?;
        json::parse(&text).map_err(|_| format!("Invalid {} ownership record", self.label))
    }

    fn inspect(self, path: &Path) -> Result<Metadata, String> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("Could not inspect {} path: {error}", self.label))?;
        if super::linked(&metadata) {
            return Err(format!(
                "{} path contains a link or junction; files were left unchanged",
                self.label
            ));
        }
        Ok(metadata)
    }
}

/// `boundary` is the outermost ancestor this store owns and must recheck.
/// Output checks every ancestor; temporary areas check their four owned levels.
pub struct Directory {
    path: PathBuf,
    boundary: PathBuf,
    marker: Marker,
    identity: Value,
}

impl Directory {
    pub fn new(path: PathBuf, boundary: PathBuf, marker: Marker, identity: Value) -> Self {
        Self {
            path,
            boundary,
            marker,
            identity,
        }
    }

    /// Establish ownership only after the caller has created its private paths.
    pub fn ensure(&self) -> Result<(), String> {
        self.require_ancestors(&self.path)?;
        let marker = self.path.join(self.marker.name);
        if super::metadata_if_present(&marker)
            .map_err(|error| error.to_string())?
            .is_none()
        {
            if fs::read_dir(&self.path)
                .map_err(|error| error.to_string())?
                .next()
                .is_some()
            {
                return Err(format!(
                    "{} directory has no ownership record; existing files were left unchanged",
                    self.marker.label
                ));
            }
            self.write_marker()?;
        }
        self.validate()
    }

    fn validate(&self) -> Result<(), String> {
        if self.marker.read(&self.path)? != self.identity {
            return Err(format!(
                "{} directory belongs to another folder or session; files were left unchanged",
                self.marker.label
            ));
        }
        Ok(())
    }

    fn write_marker(&self) -> Result<(), String> {
        let path = self.path.join(self.marker.name);
        let mut file = super::private_options()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                format!(
                    "Could not establish {} ownership: {error}",
                    self.marker.label
                )
            })?;
        let result = file
            .write_all(self.identity.encode().as_bytes())
            .and_then(|()| file.sync_all());
        drop(file);
        if let Err(error) = result {
            let _ = fs::remove_file(&path);
            return Err(format!(
                "Could not save {} ownership: {error}",
                self.marker.label
            ));
        }
        Ok(())
    }

    fn ancestors_present(&self, path: &Path) -> Result<bool, String> {
        if !path.starts_with(&self.boundary) {
            return Err("Owned path is outside its storage boundary".into());
        }
        let ancestors: Vec<_> = path
            .ancestors()
            .take_while(|parent| Some(*parent) != self.boundary.parent())
            .collect();
        for parent in ancestors.into_iter().rev() {
            let Some(metadata) = super::metadata_if_present(parent).map_err(|error| {
                format!("Could not inspect {} directory: {error}", self.marker.label)
            })?
            else {
                return Ok(false);
            };
            if !super::ordinary_directory(&metadata) {
                return Err(format!(
                    "{} path contains a link, junction or non-directory entry; files were left unchanged",
                    self.marker.label
                ));
            }
        }
        Ok(true)
    }

    fn require_ancestors(&self, path: &Path) -> Result<(), String> {
        if !self.ancestors_present(path)? {
            return Err(format!("{} directory disappeared", self.marker.label));
        }
        Ok(())
    }

    /// Inspect the complete tree before removing anything. Contents precede
    /// their directories; the ownership marker and root are always last.
    pub fn prepare_removal(self) -> Result<Option<Removal>, String> {
        if !self.ancestors_present(&self.path)? {
            return Ok(None);
        }
        self.validate()?;
        let marker = self.path.join(self.marker.name);
        let mut entries = Vec::new();
        let mut pending = vec![(self.path.clone(), false)];
        let mut files = 0;
        let mut directories = 0;
        let mut bytes = Some(0u64);
        while let Some((path, visited)) = pending.pop() {
            self.require_ancestors(path.parent().ok_or("Invalid owned path")?)?;
            let metadata = self.marker.inspect(&path)?;
            if (visited || path == self.path) && !metadata.is_dir() {
                return Err("An owned directory changed during inspection".into());
            }
            if metadata.is_file() {
                files += 1;
                bytes = bytes.and_then(|bytes| bytes.checked_add(metadata.len()));
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
                    for entry in fs::read_dir(&path).map_err(|error| error.to_string())? {
                        let entry = entry.map_err(|error| error.to_string())?.path();
                        if entry != marker {
                            pending.push((entry, false));
                        }
                    }
                }
            } else {
                return Err(format!(
                    "{} directory contains a special file; files were left unchanged",
                    self.marker.label
                ));
            }
        }
        Ok(Some(Removal {
            owner: self,
            entries,
            files,
            directories,
            bytes,
        }))
    }

    fn restore_marker(&self) -> Result<(), String> {
        self.require_ancestors(&self.path)?;
        self.write_marker()?;
        self.require_ancestors(&self.path)?;
        self.validate()
    }
}

pub struct Removal {
    owner: Directory,
    entries: Vec<(PathBuf, bool)>,
    files: usize,
    directories: usize,
    bytes: Option<u64>,
}

#[derive(Debug, Default)]
pub struct Removed {
    pub files: usize,
    pub directories: usize,
}

impl Removal {
    /// Counts for cleanup, excluding the retained marker and root.
    pub fn contents(&self) -> Result<(usize, usize, u64), String> {
        Ok((
            self.files - 1,
            self.directories - 1,
            self.bytes
                .ok_or_else(|| format!("{} byte count overflow", self.owner.marker.label))?,
        ))
    }

    pub fn remove(self) -> Result<Removed, String> {
        self.execute(false)
    }

    /// Empty the area while keeping its ownership record and root directory.
    pub fn clean(self) -> Result<(), String> {
        self.execute(true).map(|_| ())
    }

    fn execute(self, keep_root: bool) -> Result<Removed, String> {
        let owner = &self.owner;
        let marker = owner.path.join(owner.marker.name);
        for (removed, (path, directory)) in self
            .entries
            .iter()
            .filter(|(path, _)| !keep_root || (path != &owner.path && path != &marker))
            .enumerate()
        {
            let result = (|| {
                // Recheck the entire owned ancestry before reading the marker
                // or using a prepared path; a parent may have been replaced.
                owner.require_ancestors(&owner.path)?;
                if path != &owner.path {
                    owner.validate()?;
                }
                owner.require_ancestors(path.parent().ok_or("Invalid owned path")?)?;
                let metadata = owner.marker.inspect(path)?;
                if metadata.is_dir() != *directory || !(metadata.is_dir() || metadata.is_file()) {
                    return Err("An owned entry changed during removal".into());
                }
                let result = if *directory {
                    fs::remove_dir(path)
                } else {
                    fs::remove_file(path)
                };
                if let Err(error) = result {
                    if path == &owner.path {
                        return match owner.restore_marker() {
                            Ok(()) => Err(format!(
                                "{error}. {} ownership was restored for a retry",
                                owner.marker.label
                            )),
                            Err(restore_error) => Err(format!(
                                "{error}. Could not restore {} ownership: {restore_error}",
                                owner.marker.label
                            )),
                        };
                    }
                    return Err(error.to_string());
                }
                Ok(())
            })();
            result.map_err(|error| {
                format!(
                    "{} deletion is incomplete after {removed} entries: {error}",
                    owner.marker.label
                )
            })?;
        }
        Ok(Removed {
            files: self.files,
            directories: self.directories,
        })
    }
}
