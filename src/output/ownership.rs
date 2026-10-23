use crate::disk::owned::{self, Marker};
use crate::json::Value;
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};

const MARKER: Marker = Marker {
    name: ".jecode-output.json",
    format: "jecode.output",
    label: "Output",
    max_bytes: Some(4096),
};

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

    fn directory(&self, root: &Path) -> owned::Directory {
        owned::Directory::new(
            root.join(&self.session),
            root.ancestors().last().unwrap().to_path_buf(),
            MARKER,
            MARKER.identity(&self.project, &self.session),
        )
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
    identity.directory(root).ensure()?;
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
    if !inspect(root)?.is_dir() {
        return Err("Saved output root must be a directory".into());
    }
    let directory = if let Some(session) = session {
        let path = root.join(session);
        if !inspect(&path)?.is_dir() {
            return Err("Saved output session must be a directory".into());
        }
        let project = match owner {
            Some(owner) => owner.project.clone(),
            None => {
                let value = MARKER.read(&path)?;
                PathBuf::from(
                    value
                        .get("directory")
                        .and_then(Value::as_str)
                        .ok_or("Invalid output ownership record")?,
                )
            }
        };
        let identity = Identity::new(project, session)?;
        if MARKER.read(&path)? != MARKER.identity(&identity.project, session) {
            return Err(
                "Output directory belongs to another folder or session; files were left unchanged"
                    .into(),
            );
        }
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

pub(crate) struct Removal(Option<owned::Removal>);

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
    Ok(Removal(identity.directory(root).prepare_removal()?))
}

impl Removal {
    pub(crate) fn remove(self) -> Result<owned::Removed, String> {
        self.0
            .map_or_else(|| Ok(owned::Removed::default()), owned::Removal::remove)
    }
}

fn private_directory(path: &Path) -> Result<(), String> {
    crate::disk::ensure_private_directory(path)
        .map(|_| ())
        .map_err(|error| format!("Could not prepare output directory: {error}"))
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
    let metadata = crate::disk::metadata_if_present(path)
        .map_err(|error| format!("Could not inspect saved output: {error}"))?;
    if metadata.as_ref().is_some_and(crate::disk::linked) {
        return Err("Output path contains a link or junction; files were left unchanged".into());
    }
    Ok(metadata)
}
