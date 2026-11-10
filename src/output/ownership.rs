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

/// An unscoped store's output directory: the root itself.
pub(super) fn flat_directory(root: &Path) -> Result<PathBuf, String> {
    checked_ancestors(root)?;
    Ok(root.to_path_buf())
}

/// A session's directory under this store's root, when the session saved
/// outputs there. Its record names the owner's folder, or for an unowned
/// store the folder it records.
pub(super) fn session_directory(
    root: &Path,
    owner: Option<&Identity>,
    session: &str,
) -> Result<Option<PathBuf>, String> {
    let Some(path) = directory(root, session)? else {
        return Ok(None);
    };
    let project = match owner {
        Some(owner) => owner.project.clone(),
        None => recorded_project(&path)?,
    };
    let identity = Identity::new(project, session)?;
    verify(&path, &identity.project, session)?;
    Ok(Some(path))
}

/// The own directory of an agent above this one in its conversation.
pub(super) fn owned_directory(root: &Path, owner: &Identity) -> Result<Option<PathBuf>, String> {
    let Some(path) = directory(root, &owner.session)? else {
        return Ok(None);
    };
    verify(&path, &owner.project, &owner.session)?;
    Ok(Some(path))
}

/// A collaborator's directory in the home where this conversation's
/// collaborators keep their sessions: `sessions/<folder>/outputs/<session>`.
/// That home belongs to this conversation alone; the record names the
/// collaborator's session and the folder it worked in.
pub(super) fn collaborator_directory(
    home: &Path,
    session: &str,
) -> Result<Option<PathBuf>, String> {
    let sessions = home.join("sessions");
    checked_ancestors(&sessions)?;
    if !inspect_optional(&sessions)?.is_some_and(|metadata| metadata.is_dir()) {
        return Ok(None);
    }
    let entries = fs::read_dir(&sessions)
        .map_err(|error| format!("Could not inspect saved output: {error}"))?;
    for entry in entries {
        let folder = entry
            .map_err(|error| format!("Could not inspect saved output: {error}"))?
            .path();
        let root = folder.join("outputs");
        let present = crate::disk::metadata_if_present(&folder)
            .ok()
            .flatten()
            .is_some_and(|metadata| crate::disk::ordinary_directory(&metadata))
            && crate::disk::metadata_if_present(&root.join(session))
                .ok()
                .flatten()
                .is_some();
        if !present {
            continue;
        }
        if let Some(path) = directory(&root, session)? {
            verify(&path, &recorded_project(&path)?, session)?;
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// The saved stream file, when it still exists.
pub(super) fn saved_file(
    directory: &Path,
    id: &str,
    stream: &str,
) -> Result<Option<PathBuf>, String> {
    let path = directory.join(format!("{id}.{stream}"));
    match inspect_optional(&path)? {
        None => Ok(None),
        Some(metadata) if metadata.is_file() => Ok(Some(path)),
        Some(_) => Err("Output references must name a saved regular file".into()),
    }
}

/// `root/session` when it exists, refusing links and other entries on the way.
fn directory(root: &Path, session: &str) -> Result<Option<PathBuf>, String> {
    checked_ancestors(root)?;
    if inspect_optional(root)?.is_none() {
        return Ok(None);
    }
    let path = root.join(session);
    match inspect_optional(&path)? {
        None => Ok(None),
        Some(metadata) if metadata.is_dir() => Ok(Some(path)),
        Some(_) => Err("Saved output session must be a directory".into()),
    }
}

fn recorded_project(directory: &Path) -> Result<PathBuf, String> {
    MARKER
        .read(directory)?
        .get("directory")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| "Invalid output ownership record".into())
}

fn verify(directory: &Path, project: &Path, session: &str) -> Result<(), String> {
    if MARKER.read(directory)? != MARKER.identity(project, session) {
        return Err(
            "Output directory belongs to another folder or session; files were left unchanged"
                .into(),
        );
    }
    Ok(())
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

fn inspect_optional(path: &Path) -> Result<Option<Metadata>, String> {
    let metadata = crate::disk::metadata_if_present(path)
        .map_err(|error| format!("Could not inspect saved output: {error}"))?;
    if metadata.as_ref().is_some_and(crate::disk::linked) {
        return Err("Output path contains a link or junction; files were left unchanged".into());
    }
    Ok(metadata)
}
