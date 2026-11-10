use crate::disk::owned::{self, Marker};
use crate::json::Value;
use crate::sessions::layout;
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

    fn directory(&self, sessions: &Path) -> owned::Directory {
        owned::Directory::new(
            outputs(sessions, &self.session),
            sessions.ancestors().last().unwrap().to_path_buf(),
            MARKER,
            MARKER.identity(&self.project, &self.session),
        )
    }
}

/// Where `session` saves outputs among the session folders in `sessions`.
fn outputs(sessions: &Path, session: &str) -> PathBuf {
    sessions.join(session).join(layout::OUTPUTS)
}

/// Creates the session's own output directory among the session folders in
/// `sessions`, with its ownership record.
pub(super) fn ensure(sessions: &Path, identity: &Identity) -> Result<PathBuf, String> {
    if !sessions.is_absolute() {
        return Err("Output directory must be absolute".into());
    }
    let path = outputs(sessions, &identity.session);
    for directory in [sessions, &sessions.join(&identity.session), &path] {
        checked_ancestors(directory)?;
        private_directory(directory)?;
    }
    identity.directory(sessions).ensure()?;
    Ok(path)
}

/// An unscoped store's output directory: the root itself.
pub(super) fn flat_directory(root: &Path) -> Result<PathBuf, String> {
    checked_ancestors(root)?;
    Ok(root.to_path_buf())
}

/// The output directory of a session of this store's project, when the
/// session saved outputs. Its record names the owner's folder.
pub(super) fn session_directory(
    sessions: &Path,
    owner: Option<&Identity>,
    session: &str,
) -> Result<Option<PathBuf>, String> {
    let Some(path) = directory(sessions, session)? else {
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
pub(super) fn owned_directory(
    sessions: &Path,
    owner: &Identity,
) -> Result<Option<PathBuf>, String> {
    let Some(path) = directory(sessions, &owner.session)? else {
        return Ok(None);
    };
    verify(&path, &owner.project, &owner.session)?;
    Ok(Some(path))
}

/// A collaborator's directory in the home where this conversation's
/// collaborators keep their sessions: `sessions/<folder>/<session>/outputs`.
/// That home belongs to this conversation alone; the record names the
/// collaborator's session and the folder it worked in.
pub(super) fn collaborator_directory(
    home: &Path,
    session: &str,
) -> Result<Option<PathBuf>, String> {
    let sessions = home.join(layout::SESSIONS);
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
        let present = crate::disk::metadata_if_present(&folder)
            .ok()
            .flatten()
            .is_some_and(|metadata| crate::disk::ordinary_directory(&metadata))
            && crate::disk::metadata_if_present(&outputs(&folder, session))
                .ok()
                .flatten()
                .is_some();
        if !present {
            continue;
        }
        if let Some(path) = directory(&folder, session)? {
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

/// `sessions/session/outputs` when it exists, refusing links and other
/// entries on the way.
fn directory(sessions: &Path, session: &str) -> Result<Option<PathBuf>, String> {
    checked_ancestors(sessions)?;
    if inspect_optional(sessions)?.is_none() {
        return Ok(None);
    }
    for path in [sessions.join(session), outputs(sessions, session)] {
        match inspect_optional(&path)? {
            None => return Ok(None),
            Some(metadata) if metadata.is_dir() => {}
            Some(_) => return Err("Saved output session must be a directory".into()),
        }
    }
    Ok(Some(outputs(sessions, session)))
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

/// The removal of `session`'s output directory among the session folders in
/// `sessions`.
pub(crate) fn prepare_removal(
    sessions: &Path,
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
    if !sessions.is_absolute() {
        return Err("Output directory must be absolute".into());
    }
    Ok(Removal(identity.directory(sessions).prepare_removal()?))
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
