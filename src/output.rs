use crate::redact::Redactor;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

mod ownership;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tree_tests;

pub(crate) use ownership::{Removal, prepare_removal};

#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    redactor: Redactor,
    owner: Option<ownership::Identity>,
    establishment: Arc<Mutex<()>>,
    /// The agents above this one in its conversation, nearest first: a
    /// collaborator reads the outputs its main agent hands it.
    ancestors: Vec<(PathBuf, ownership::Identity)>,
    /// Where the collaborators this conversation starts keep their sessions:
    /// the main agent reads the outputs they report.
    collaborators: Option<PathBuf>,
}

pub struct Logs {
    pub stdout: Spool,
    pub stderr: Spool,
    pub stdout_ref: String,
    pub stderr_ref: String,
}

pub struct Spool {
    file: File,
    redactor: Redactor,
    pending: Vec<u8>,
}

impl Store {
    pub fn new(root: PathBuf, redactor: Redactor) -> Self {
        Self {
            root,
            redactor,
            owner: None,
            establishment: Arc::new(Mutex::new(())),
            ancestors: Vec::new(),
            collaborators: None,
        }
    }

    pub fn for_session(
        root: PathBuf,
        project: PathBuf,
        session: &str,
        redactor: Redactor,
    ) -> Result<Self, String> {
        if !root.is_absolute() {
            return Err("Output directory must be absolute".into());
        }
        Ok(Self {
            root,
            redactor,
            owner: Some(ownership::Identity::new(project, session)?),
            establishment: Arc::new(Mutex::new(())),
            ancestors: Vec::new(),
            collaborators: None,
        })
    }

    /// Also resolves the outputs of collaborators whose sessions live in `home`.
    pub fn with_collaborators(mut self, home: PathBuf) -> Self {
        self.collaborators = Some(home);
        self
    }

    /// Lets a collaborator read the outputs of the agents that delegated to it.
    pub fn inherit(&mut self, parent: &Store) {
        if let Some(owner) = &parent.owner {
            self.ancestors = std::iter::once((parent.root.clone(), owner.clone()))
                .chain(parent.ancestors.iter().cloned())
                .collect();
        }
    }
    pub fn include(&mut self, redactor: Redactor) {
        self.redactor.include(redactor);
    }

    fn owned_directory(&self) -> Result<PathBuf, String> {
        let _establishment = self.establishment.lock().unwrap_or_else(|e| e.into_inner());
        Ok(if let Some(owner) = &self.owner {
            ownership::ensure(&self.root, owner)?
        } else {
            crate::disk::private_directory(&self.root)
                .map_err(|error| format!("Could not create tool output directory: {error}"))?;
            self.root.clone()
        })
    }

    pub fn create(&self) -> Result<Logs, String> {
        let directory = self.owned_directory()?;
        let id = crate::sessions::identifier();
        let make = |stream: &str| {
            let file = crate::disk::private_options()
                .write(true)
                .create_new(true)
                .open(directory.join(format!("{id}.{stream}")))
                .map_err(|error| format!("Could not save tool output: {error}"))?;
            Ok::<_, String>(Spool {
                file,
                redactor: self.redactor.clone(),
                pending: vec![],
            })
        };
        Ok(Logs {
            stdout: make("stdout")?,
            stderr: make("stderr")?,
            stdout_ref: self.reference(&id, "stdout"),
            stderr_ref: self.reference(&id, "stderr"),
        })
    }

    fn reference(&self, id: &str, stream: &str) -> String {
        match &self.owner {
            Some(owner) => format!("output:{}:{id}:{stream}", owner.session),
            None => format!("output:{id}:{stream}"),
        }
    }

    /// The saved file a reference names. References resolve within one
    /// conversation tree: this store's sessions, the agents above it and the
    /// collaborators it starts; never another conversation's collaborators.
    pub fn resolve(&self, reference: &str) -> Result<PathBuf, String> {
        let (session, id, stream) = parse(reference)
            .ok_or("Invalid output reference: pass the exact output: value a tool returned")?;
        let directory = match session {
            None => ownership::flat_directory(&self.root)?,
            Some(session) => self.session_directory(session)?.ok_or_else(|| {
                format!(
                    "{reference} is not saved for this conversation or its agents; ask the agent that ran it for the content, or run the command again"
                )
            })?,
        };
        ownership::saved_file(&directory, id, stream)?.ok_or_else(|| {
            format!(
                "{reference} is no longer saved; run the command again if its output is still needed"
            )
        })
    }

    fn session_directory(&self, session: &str) -> Result<Option<PathBuf>, String> {
        if let Some((root, owner)) = self
            .ancestors
            .iter()
            .find(|(_, owner)| owner.session == session)
        {
            return ownership::owned_directory(root, owner);
        }
        if let Some(directory) =
            ownership::session_directory(&self.root, self.owner.as_ref(), session)?
        {
            return Ok(Some(directory));
        }
        match &self.collaborators {
            Some(home) => ownership::collaborator_directory(home, session),
            None => Ok(None),
        }
    }
}

/// The optional session, the output id and the stream a reference names.
fn parse(reference: &str) -> Option<(Option<&str>, &str, &str)> {
    let parts: Vec<_> = reference.strip_prefix("output:")?.split(':').collect();
    let (session, id, stream) = match parts.as_slice() {
        [id, stream] => (None, *id, *stream),
        [session, id, stream] => (Some(*session), *id, *stream),
        _ => return None,
    };
    (crate::sessions::valid_id(id)
        && matches!(stream, "stdout" | "stderr")
        && session.is_none_or(crate::sessions::valid_id))
    .then_some((session, id, stream))
}

impl Spool {
    #[cfg(test)]
    pub fn read_only_fixture(path: &std::path::Path) -> Self {
        Self {
            file: File::open(path).unwrap(),
            redactor: Redactor::empty(),
            pending: vec![],
        }
    }

    pub fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.pending.extend_from_slice(bytes);
        let (consumed, redacted) = self.redactor.byte_prefix(&self.pending, false);
        self.file.write_all(&redacted)?;
        self.pending.drain(..consumed);
        Ok(())
    }
    pub fn finish(&mut self) -> std::io::Result<()> {
        let (_, redacted) = self.redactor.byte_prefix(&self.pending, true);
        self.file.write_all(&redacted)?;
        self.pending.clear();
        self.file.sync_all()
    }
}

pub fn is_reference(path: &str) -> bool {
    path.starts_with("output:")
}
