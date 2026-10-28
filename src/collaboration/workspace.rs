//! Assignment working copies and evidence for coordinator-directed integration.
use crate::json::{self, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

mod files;
mod git;
mod review;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Shared,
    Isolated,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Isolated => "isolated",
        }
    }
}

#[derive(Clone)]
pub struct Workspace {
    project: PathBuf,
    storage: PathBuf,
    id: String,
    mode: Mode,
    root: PathBuf,
    baseline: BTreeMap<String, files::Fingerprint>,
}

impl Workspace {
    pub fn prepare(
        project_root: &Path,
        storage_root: &Path,
        id: &str,
        mode: Mode,
    ) -> Result<Self, String> {
        let project = fs::canonicalize(project_root)
            .map_err(|error| format!("Could not resolve project: {error}"))?;
        if !project.is_dir() {
            return Err("Project root must be a directory".into());
        }
        if mode == Mode::Shared {
            return Ok(Self {
                project: project.clone(),
                storage: storage_root.to_path_buf(),
                id: id.into(),
                mode,
                root: project,
                baseline: BTreeMap::new(),
            });
        }
        valid_id(id)?;
        crate::disk::ensure_private_directory(storage_root)
            .map_err(|error| format!("Could not create collaboration storage: {error}"))?;
        let storage_root = fs::canonicalize(storage_root)
            .map_err(|error| format!("Could not resolve collaboration storage: {error}"))?;
        if project.starts_with(&storage_root) || storage_root.starts_with(&project) {
            return Err("Isolated storage must be outside the project tree".into());
        }
        git::require_repository(&project)?;
        let storage = storage_root.join(id);
        let root = storage.join("tree");
        let mut workspace = Self {
            project,
            storage,
            id: id.into(),
            mode,
            root,
            baseline: BTreeMap::new(),
        };
        match fs::create_dir(&workspace.storage) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if workspace.marker_ready()? {
                    return Err(format!(
                        "Assignment workspace {} is complete and must be closed explicitly",
                        workspace.storage.display()
                    ));
                }
                workspace.close()?;
                fs::create_dir(&workspace.storage).map_err(|error| {
                    format!("Could not recreate recovered assignment storage: {error}")
                })?;
            }
            Err(error) => return Err(format!("Could not create assignment storage: {error}")),
        }
        if let Err(error) = workspace.save(false) {
            return match fs::remove_dir(&workspace.storage) {
                Ok(()) => Err(error),
                Err(cleanup) => Err(format!(
                    "{error}; could not remove new assignment directory {}: {cleanup}",
                    workspace.storage.display()
                )),
            };
        }
        let result = (|| {
            git::create_worktree(&workspace.project, &workspace.root)?;
            let base = workspace.storage.join("base");
            fs::create_dir(&base).map_err(|error| format!("Could not create baseline: {error}"))?;
            let source = git::inventory(&workspace.project)?;
            let source = git::with_instructions(&workspace.project, source)?;
            let initial_child = git::inventory(&workspace.root)?;
            let names: BTreeSet<_> = source.union(&initial_child).cloned().collect();
            let mut baseline = BTreeMap::new();
            let mut observed = BTreeMap::new();
            for name in &names {
                files::validate_name(name)?;
                let original = files::observe(&workspace.project, name)?;
                files::copy_entry(&workspace.project, &workspace.root, name, &original)?;
                files::save_baseline(&workspace.project, &base, name, &original)?;
                if original.exists() {
                    baseline.insert(name.clone(), original.clone());
                }
                observed.insert(name.clone(), original);
            }
            if git::with_instructions(&workspace.project, git::inventory(&workspace.project)?)?
                != source
            {
                return Err("Project file inventory changed during workspace preparation".into());
            }
            for (name, original) in &observed {
                if files::observe(&workspace.project, name)? != *original {
                    return Err(format!("{name} changed during workspace preparation"));
                }
            }
            workspace.baseline = baseline;
            workspace.save(true)?;
            Ok(())
        })();
        if let Err(error) = result {
            return match workspace.close() {
                Ok(_) => Err(error),
                Err(cleanup) => Err(format!(
                    "{error}; setup cleanup failed at {}: {cleanup}",
                    workspace.storage.display()
                )),
            };
        }
        Ok(workspace)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn mode(&self) -> Mode {
        self.mode
    }
    pub fn inspect(&self) -> Result<Value, String> {
        review::inspect(self)?.value(self)
    }
    pub fn inspect_file(&self, path: &str, offset: usize, limit: usize) -> Result<Value, String> {
        review::inspect_file(self, path, offset, limit)
    }
    pub fn integrate(
        &self,
        paths: &[String],
        expected_review_token: &str,
    ) -> Result<Value, String> {
        review::integrate(self, paths, expected_review_token)
    }

    pub fn close(&self) -> Result<Value, String> {
        if self.mode == Mode::Shared {
            return Ok(Value::object([("closed", Value::Bool(true))]));
        }
        self.verify_owner()?;
        if self.root.exists() || git::registered(&self.project, &self.root)? {
            git::remove_worktree(&self.project, &self.root)?;
        }
        let base = self.storage.join("base");
        if base.exists() {
            files::remove_baseline(&base)?;
        }
        fs::remove_file(self.storage.join("baseline.json"))
            .map_err(|error| format!("Worktree removed; could not remove baseline: {error}"))?;
        fs::remove_dir(&self.storage).map_err(|error| {
            format!("Worktree removed; could not remove assignment directory: {error}")
        })?;
        Ok(Value::object([("closed", Value::Bool(true))]))
    }

    pub fn value(&self) -> Value {
        Value::object([
            ("project", Value::string(self.project.to_string_lossy())),
            ("storage", Value::string(self.storage.to_string_lossy())),
            ("id", Value::string(&self.id)),
            ("mode", Value::string(self.mode.label())),
        ])
    }

    pub fn from_value(value: &Value) -> Result<Self, String> {
        let field = |key| {
            value
                .get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("Missing workspace {key}"))
        };
        let project = PathBuf::from(field("project")?);
        let storage = PathBuf::from(field("storage")?);
        let id = field("id")?.to_owned();
        let mode = match field("mode")? {
            "shared" => Mode::Shared,
            "isolated" => Mode::Isolated,
            _ => return Err("Invalid workspace mode".into()),
        };
        if mode == Mode::Shared {
            let project = fs::canonicalize(project).map_err(|error| error.to_string())?;
            return Ok(Self {
                root: project.clone(),
                project,
                storage,
                id,
                mode,
                baseline: BTreeMap::new(),
            });
        }
        valid_id(&id)?;
        let root = storage.join("tree");
        let mut workspace = Self {
            project,
            storage,
            id,
            mode,
            root,
            baseline: BTreeMap::new(),
        };
        workspace.verify_owner()?;
        let bytes = fs::read(workspace.storage.join("baseline.json"))
            .map_err(|error| format!("Could not read workspace baseline: {error}"))?;
        let value =
            json::parse(&String::from_utf8(bytes).map_err(|_| "Invalid baseline encoding")?)
                .map_err(|error| format!("Invalid workspace baseline: {error}"))?;
        if value.get("ready") != Some(&Value::Bool(true)) {
            return Err(
                "Workspace preparation was interrupted; its owned directory needs cleanup".into(),
            );
        }
        let entries = value
            .get("entries")
            .and_then(|value| match value {
                Value::Object(entries) => Some(entries),
                _ => None,
            })
            .ok_or("Missing workspace baseline entries")?;
        for (name, value) in entries {
            files::validate_name(name)?;
            workspace
                .baseline
                .insert(name.clone(), files::Fingerprint::from_value(value)?);
        }
        Ok(workspace)
    }

    fn save(&self, ready: bool) -> Result<(), String> {
        let entries = self
            .baseline
            .iter()
            .map(|(name, state)| (name.clone(), state.value()))
            .collect();
        let value = Value::object([
            ("owner", self.value()),
            ("ready", Value::Bool(ready)),
            ("entries", Value::Object(entries)),
        ]);
        crate::disk::atomic_write(
            &self.storage.join("baseline.json"),
            value.encode().as_bytes(),
        )
        .map_err(|error| format!("Could not save workspace baseline: {error}"))
    }

    fn verify_owner(&self) -> Result<(), String> {
        valid_id(&self.id)?;
        if self.storage.file_name().and_then(|part| part.to_str()) != Some(self.id.as_str())
            || self.root != self.storage.join("tree")
        {
            return Err("Workspace ownership path is invalid".into());
        }
        let storage = fs::canonicalize(&self.storage).map_err(|error| error.to_string())?;
        if storage != self.storage
            || !crate::disk::ordinary_directory(
                &fs::symlink_metadata(&storage).map_err(|error| error.to_string())?,
            )
        {
            return Err("Workspace storage is not an ordinary owned directory".into());
        }
        if fs::canonicalize(&self.project).map_err(|error| error.to_string())? != self.project {
            return Err("Workspace project identity changed".into());
        }
        for path in [&self.root, &self.storage.join("base")] {
            if let Some(metadata) =
                crate::disk::metadata_if_present(path).map_err(|error| error.to_string())?
                && !crate::disk::ordinary_directory(&metadata)
            {
                return Err(format!(
                    "Workspace owned path is a link or non-directory: {}",
                    path.display()
                ));
            }
        }
        let marker = self.storage.join("baseline.json");
        let metadata = fs::symlink_metadata(&marker).map_err(|error| error.to_string())?;
        if !metadata.is_file() || crate::disk::linked(&metadata) {
            return Err("Workspace ownership marker is not a regular file".into());
        }
        let bytes = fs::read(marker)
            .map_err(|error| format!("Could not read workspace ownership: {error}"))?;
        let value =
            json::parse(&String::from_utf8(bytes).map_err(|_| "Invalid ownership encoding")?)
                .map_err(|error| error.to_string())?;
        if value.get("owner") != Some(&self.value()) {
            return Err("Workspace belongs to another project or assignment".into());
        }
        Ok(())
    }

    fn marker_ready(&self) -> Result<bool, String> {
        self.verify_owner()?;
        let bytes = fs::read(self.storage.join("baseline.json"))
            .map_err(|error| format!("Could not read workspace state: {error}"))?;
        let value =
            json::parse(&String::from_utf8(bytes).map_err(|_| "Invalid workspace state encoding")?)
                .map_err(|error| error.to_string())?;
        match value.get("ready") {
            Some(Value::Bool(ready)) => Ok(*ready),
            _ => Err("Existing assignment workspace has no valid preparation state".into()),
        }
    }
}

fn valid_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("Invalid assignment workspace ID".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
