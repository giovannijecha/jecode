use crate::json::{self, Value};
use std::collections::BTreeSet;
use std::fs::{self, Metadata, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

const MARKER: &str = ".jecode-output.json";

#[derive(Clone)]
pub(super) struct Identity {
    project: PathBuf,
    pub session: String,
}

impl Identity {
    pub fn new(project: PathBuf, session: &str) -> Result<Self, String> {
        if !crate::sessions::valid_id(session) {
            return Err("Invalid output session identifier".into());
        }
        let project = fs::canonicalize(project)
            .map_err(|error| format!("Could not resolve output project: {error}"))?;
        if !project.is_dir() {
            return Err("Output project must be a directory".into());
        }
        Ok(Self {
            project,
            session: session.into(),
        })
    }

    fn value(&self) -> Value {
        Value::object([
            ("format", Value::string("jecode.output")),
            ("format_version", Value::number(1)),
            ("directory", Value::string(self.project.to_string_lossy())),
            ("session_id", Value::string(&self.session)),
        ])
    }
}

pub(super) fn ensure(root: &Path, identity: &Identity) -> Result<PathBuf, String> {
    if !root.is_absolute() {
        return Err("Output directory must be absolute".into());
    }
    checked_ancestors(root)?;
    private_directory(root)?;
    let session = root.join(&identity.session);
    checked_ancestors(&session)?;
    private_directory(&session)?;
    let marker = session.join(MARKER);
    if inspect_optional(&marker)?.is_none() {
        if fs::read_dir(&session)
            .map_err(|error| format!("Could not inspect saved output: {error}"))?
            .next()
            .is_some()
        {
            return Err(
                "Output directory has no ownership record; files were left unchanged".into(),
            );
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
            .map_err(|error| format!("Could not establish output ownership: {error}"))?;
        let result = file
            .write_all(identity.value().encode().as_bytes())
            .and_then(|()| file.sync_all());
        drop(file);
        if let Err(error) = result {
            let _ = fs::remove_file(&marker);
            return Err(format!("Could not save output ownership: {error}"));
        }
    }
    validate_marker(&session, identity)?;
    Ok(session)
}

pub(super) fn resolve(
    root: &Path,
    owner: Option<&Identity>,
    session: Option<&str>,
    id: &str,
    stream: &str,
) -> Result<PathBuf, String> {
    checked_ancestors(root)?;
    let root_metadata = inspect(root)?;
    if !root_metadata.is_dir() {
        return Err("Saved output root must be a directory".into());
    }
    let directory = if let Some(session) = session {
        let path = root.join(session);
        if !inspect(&path)?.is_dir() {
            return Err("Saved output session must be a directory".into());
        }
        let project = match owner {
            Some(owner) => owner.project.clone(),
            None => marker_project(&path)?,
        };
        validate_marker(&path, &Identity::new(project, session)?)?;
        path
    } else {
        root.to_path_buf()
    };
    let path = directory.join(format!("{id}.{stream}"));
    if !inspect(&path)?.is_file() {
        return Err("Output references must name a saved regular file".into());
    }
    Ok(path)
}

pub(crate) struct Removal {
    root: PathBuf,
    identity: Identity,
    files: Vec<PathBuf>,
    directories: Vec<PathBuf>,
    legacy_references: usize,
    present: bool,
}

pub(crate) struct Removed {
    pub files: usize,
    pub directories: usize,
    pub legacy_references: usize,
}

pub(crate) fn prepare_removal(
    root: &Path,
    project: &Path,
    session: &str,
    document: &crate::sessions::Document,
) -> Result<Removal, String> {
    let identity = Identity::new(project.to_path_buf(), session)?;
    if document.id != session
        || fs::canonicalize(&document.directory).ok().as_ref() != Some(&identity.project)
    {
        return Err("Output removal identity differs from the session".into());
    }
    if !root.is_absolute() {
        return Err("Output directory must be absolute".into());
    }
    checked_ancestors(root)?;
    let legacy_references = legacy_references(root, document)?;
    let mut removal = Removal {
        root: root.to_path_buf(),
        identity,
        files: Vec::new(),
        directories: Vec::new(),
        legacy_references,
        present: false,
    };
    if inspect_optional(root)?.is_none() {
        return Ok(removal);
    }
    if !inspect(root)?.is_dir() {
        return Err("Output root must be a directory".into());
    }
    let session_dir = root.join(session);
    let Some(metadata) = inspect_optional(&session_dir)? else {
        return Ok(removal);
    };
    if !metadata.is_dir() {
        return Err("Owned output path must be a directory".into());
    }
    validate_marker(&session_dir, &removal.identity)?;
    inventory(&session_dir, &mut removal.files, &mut removal.directories)?;
    removal.present = true;
    Ok(removal)
}

impl Removal {
    pub(crate) fn remove(self) -> Result<Removed, String> {
        if !self.present {
            return Ok(Removed {
                files: 0,
                directories: 0,
                legacy_references: self.legacy_references,
            });
        }
        checked_ancestors(&self.root)?;
        let session_dir = self.root.join(&self.identity.session);
        validate_marker(&session_dir, &self.identity)?;
        let mut files = 0;
        let marker = session_dir.join(MARKER);
        for path in self.files.iter().filter(|path| *path != &marker) {
            checked_ancestors(path.parent().ok_or("Invalid output path")?)?;
            if !inspect(path)?.is_file() {
                return Err("Output entry changed during removal".into());
            }
            fs::remove_file(path)
                .map_err(|error| format!("Could not remove saved output: {error}"))?;
            files += 1;
        }
        for path in self.directories.iter().filter(|path| *path != &session_dir) {
            checked_ancestors(path)?;
            fs::remove_dir(path)
                .map_err(|error| format!("Could not remove output directory: {error}"))?;
        }
        validate_marker(&session_dir, &self.identity)?;
        fs::remove_file(&marker)
            .map_err(|error| format!("Could not remove output ownership: {error}"))?;
        files += 1;
        checked_ancestors(&session_dir)?;
        if let Err(error) = fs::remove_dir(&session_dir) {
            return match restore_marker(&session_dir, &self.identity) {
                Ok(()) => Err(format!(
                    "Could not remove output directory: {error}. Output ownership was restored for a retry"
                )),
                Err(restore_error) => Err(format!(
                    "Could not remove output directory: {error}. Could not restore output ownership: {restore_error}"
                )),
            };
        }
        Ok(Removed {
            files,
            directories: self.directories.len(),
            legacy_references: self.legacy_references,
        })
    }
}

fn restore_marker(directory: &Path, identity: &Identity) -> Result<(), String> {
    checked_ancestors(directory)?;
    if !inspect(directory)?.is_dir() {
        return Err("Owned output directory changed after cleanup failure".into());
    }
    let marker = directory.join(MARKER);
    if inspect_optional(&marker)?.is_some() {
        return Err("Output ownership path was replaced after cleanup failure".into());
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
        .map_err(|error| format!("Could not recreate output ownership record: {error}"))?;
    file.write_all(identity.value().encode().as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("Could not save output ownership record: {error}"))?;
    checked_ancestors(directory)?;
    validate_marker(directory, identity)
}

fn inventory(
    path: &Path,
    files: &mut Vec<PathBuf>,
    directories: &mut Vec<PathBuf>,
) -> Result<(), String> {
    for entry in
        fs::read_dir(path).map_err(|error| format!("Could not list output directory: {error}"))?
    {
        let entry = entry.map_err(|error| format!("Could not list output directory: {error}"))?;
        let path = entry.path();
        let metadata = inspect(&path)?;
        if metadata.is_dir() {
            inventory(&path, files, directories)?;
        } else if metadata.is_file() {
            files.push(path);
        } else {
            return Err(
                "Output directory contains a special file; files were left unchanged".into(),
            );
        }
    }
    directories.push(path.to_path_buf());
    Ok(())
}

fn legacy_references(root: &Path, document: &crate::sessions::Document) -> Result<usize, String> {
    fn collect(value: &Value, references: &mut BTreeSet<String>) {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    if matches!(key.as_str(), "stdout_file" | "stderr_file")
                        && let Some(reference) = value.as_str()
                        && let Some(rest) = reference.strip_prefix("output:")
                        && let Some((id, stream)) = rest.split_once(':')
                        && crate::sessions::valid_id(id)
                        && matches!(stream, "stdout" | "stderr")
                    {
                        references.insert(format!("{id}.{stream}"));
                    }
                    collect(value, references);
                }
            }
            Value::Array(values) => values.iter().for_each(|value| collect(value, references)),
            _ => {}
        }
    }
    let mut references = BTreeSet::new();
    for value in document.messages.iter().chain(&document.events) {
        collect(value, &mut references);
        if value.get("role").and_then(Value::as_str) == Some("tool")
            && let Some(content) = value.get("content").and_then(Value::as_str)
            && let Ok(result) = json::parse(content)
        {
            collect(&result, &mut references);
        }
    }
    let mut count = 0;
    for name in references {
        if inspect_optional(&root.join(name))?.is_some_and(|metadata| metadata.is_file()) {
            count += 1;
        }
    }
    Ok(count)
}

fn marker_project(directory: &Path) -> Result<PathBuf, String> {
    let value = read_marker(directory)?;
    let project = value
        .get("directory")
        .and_then(Value::as_str)
        .ok_or("Invalid output ownership record")?;
    Ok(PathBuf::from(project))
}

fn validate_marker(directory: &Path, identity: &Identity) -> Result<(), String> {
    if read_marker(directory)? != identity.value() {
        return Err(
            "Output directory belongs to another folder or session; files were left unchanged"
                .into(),
        );
    }
    Ok(())
}

fn read_marker(directory: &Path) -> Result<Value, String> {
    let marker = directory.join(MARKER);
    let metadata = inspect(&marker)?;
    if !metadata.is_file() || metadata.len() > 4096 {
        return Err("Output ownership record must be a regular small file".into());
    }
    let mut text = String::new();
    fs::File::open(&marker)
        .and_then(|mut file| file.read_to_string(&mut text))
        .map_err(|error| format!("Could not read output ownership: {error}"))?;
    json::parse(&text).map_err(|_| "Invalid output ownership record".into())
}

fn private_directory(path: &Path) -> Result<(), String> {
    if inspect_optional(path)?.is_none() {
        fs::create_dir_all(path)
            .map_err(|error| format!("Could not create output directory: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("Could not protect output directory: {error}"))?;
        }
    }
    if !inspect(path)?.is_dir() {
        return Err("Output path must be a directory".into());
    }
    Ok(())
}

fn checked_ancestors(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        if let Some(metadata) = inspect_optional(ancestor)?
            && !metadata.is_dir()
        {
            return Err("Output path contains a non-directory entry".into());
        }
    }
    Ok(())
}

fn inspect(path: &Path) -> Result<Metadata, String> {
    inspect_optional(path)?.ok_or_else(|| format!("Saved output is missing: {}", path.display()))
}

fn inspect_optional(path: &Path) -> Result<Option<Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            #[cfg(windows)]
            let linked = {
                use std::os::windows::fs::MetadataExt;
                metadata.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
            };
            #[cfg(not(windows))]
            let linked = metadata.file_type().is_symlink();
            if linked {
                return Err(
                    "Output path contains a link or junction; files were left unchanged".into(),
                );
            }
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Could not inspect saved output: {error}")),
    }
}
