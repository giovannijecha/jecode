//! Shared project notes, independent of conversation lifetime.
use crate::{
    cancel::Cancellation,
    disk,
    json::{self, Value},
};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const FORMAT: &str = "jecode.project-map";
const MAX_BYTES: u64 = 8 * 1024 * 1024;
const MAX_NOTES: usize = 256;

pub(super) struct Store {
    root: PathBuf,
    home: Option<PathBuf>,
}

impl Store {
    pub fn new(root: PathBuf) -> Self {
        Self { root, home: None }
    }

    pub fn configure(&mut self, home: &Path) {
        self.home = Some(home.to_path_buf());
    }

    fn directory(&self) -> Result<PathBuf, String> {
        let home = self
            .home
            .as_ref()
            .ok_or("Project map note storage is unavailable")?;
        let digest = disk::hash::digest(self.root.as_os_str().as_encoded_bytes()).0;
        Ok(home
            .join("projects")
            .join(format!("{:016x}{:016x}", digest[0], digest[1])))
    }

    fn prepare(&self, create: bool) -> Result<bool, String> {
        let directory = self.directory()?;
        let home = self.home.as_ref().unwrap();
        for path in [home.clone(), home.join("projects"), directory] {
            if create {
                disk::ensure_private_directory(&path)
                    .map_err(|error| format!("Could not prepare project map storage: {error}"))?;
            } else {
                match disk::metadata_if_present(&path).map_err(|error| error.to_string())? {
                    None => return Ok(false),
                    Some(metadata) if disk::ordinary_directory(&metadata) => (),
                    _ => {
                        return Err(
                            "Project map storage contains a link, junction or non-directory entry"
                                .into(),
                        );
                    }
                }
            }
        }
        Ok(true)
    }

    pub fn read(&self) -> Result<Vec<Value>, String> {
        if self.home.is_none() || !self.prepare(false)? {
            return Ok(Vec::new());
        }
        self.load()
    }

    fn load(&self) -> Result<Vec<Value>, String> {
        let path = self.directory()?.join("map.json");
        let Some(metadata) = disk::metadata_if_present(&path).map_err(|error| error.to_string())?
        else {
            return Ok(Vec::new());
        };
        if disk::linked(&metadata) || !metadata.is_file() || metadata.len() > MAX_BYTES {
            return Err("Project map must be a regular file of at most 8 MiB".into());
        }
        let mut bytes = Vec::new();
        fs::File::open(&path)
            .and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut bytes))
            .map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("Project map exceeds 8 MiB".into());
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| "Project map is not UTF-8")?;
        let value =
            json::parse(text).map_err(|error| format!("Could not read project map: {error}"))?;
        if value.get("format").and_then(Value::as_str) != Some(FORMAT)
            || value.get("format_version") != Some(&Value::number(1))
            || value.get("directory").and_then(Value::as_str)
                != Some(self.root.to_string_lossy().as_ref())
        {
            return Err("Project map format or workspace identity does not match; notes were left unchanged".into());
        }
        let notes = value
            .get("notes")
            .and_then(Value::as_array)
            .ok_or("Invalid project map notes")?;
        if notes.len() > MAX_NOTES {
            return Err("Project map exceeds 256 notes".into());
        }
        for note in notes {
            for key in ["id", "title", "summary"] {
                if note.get(key).and_then(Value::as_str).is_none() {
                    return Err(format!("Invalid project map note {key}"));
                }
            }
            let sources = note
                .get("sources")
                .and_then(Value::as_array)
                .ok_or("Invalid project map sources")?;
            if sources.is_empty()
                || sources.len() > 16
                || sources.iter().any(|source| {
                    ["path", "fingerprint"]
                        .into_iter()
                        .any(|key| source.get(key).and_then(Value::as_str).is_none())
                })
            {
                return Err("Invalid project map source references".into());
            }
        }
        Ok(notes.to_vec())
    }

    /// The lock covers read/modify/replace so independent processes keep each
    /// other's notes. Reads rely on the complete atomic replacement instead.
    pub fn update<T>(
        &self,
        cancellation: &Cancellation,
        change: impl FnOnce(&mut Vec<Value>) -> Result<T, String>,
    ) -> Result<T, String> {
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        self.prepare(true)?;
        let directory = self.directory()?;
        let lock = directory.join("map.lock");
        if disk::metadata_if_present(&lock)
            .map_err(|error| error.to_string())?
            .is_some_and(|metadata| disk::linked(&metadata) || !metadata.is_file())
        {
            return Err("Project map lock must be a regular file".into());
        }
        let _lock = disk::lock::Exclusive::acquire_cancellable(&lock, cancellation)
            .map_err(|error| error.to_string())?;
        self.prepare(false)?;
        let mut notes = self.load()?;
        let result = change(&mut notes)?;
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        if notes.len() > MAX_NOTES {
            return Err(
                "Project map holds at most 256 notes; update or forget an existing note".into(),
            );
        }
        let document = Value::object([
            ("format", Value::string(FORMAT)),
            ("format_version", Value::number(1)),
            ("directory", Value::string(self.root.to_string_lossy())),
            ("notes", Value::Array(notes)),
        ])
        .encode();
        if document.len() as u64 > MAX_BYTES {
            return Err("Project map exceeds the 8 MiB storage budget".into());
        }
        disk::atomic_write(&directory.join("map.json"), document.as_bytes())
            .map_err(|error| format!("Could not save project map: {error}"))?;
        Ok(result)
    }
}
