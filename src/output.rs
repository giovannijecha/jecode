use crate::redact::Redactor;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

mod ownership;
#[cfg(test)]
mod tests;

pub(crate) use ownership::{Removal, prepare_removal};

#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    redactor: Redactor,
    owner: Option<ownership::Identity>,
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
        })
    }
    pub fn include(&mut self, redactor: Redactor) {
        self.redactor.include(redactor);
    }

    fn owned_directory(&self) -> Result<PathBuf, String> {
        Ok(if let Some(owner) = &self.owner {
            ownership::ensure(&self.root, owner)?
        } else {
            if !self.root.exists() {
                fs::create_dir_all(&self.root)
                    .map_err(|error| format!("Could not create tool output directory: {error}"))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))
                        .map_err(|error| error.to_string())?;
                }
            }
            self.root.clone()
        })
    }

    pub fn create(&self) -> Result<Logs, String> {
        let directory = self.owned_directory()?;
        let id = crate::sessions::identifier();
        let make = |stream: &str| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let file = options
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

    pub fn resolve(&self, reference: &str) -> Result<PathBuf, String> {
        let rest = reference
            .strip_prefix("output:")
            .ok_or("Invalid output reference")?;
        let parts: Vec<_> = rest.split(':').collect();
        let (session, id, stream) = match parts.as_slice() {
            [id, stream] => (None, *id, *stream),
            [session, id, stream] => (Some(*session), *id, *stream),
            _ => return Err("Invalid output reference".into()),
        };
        if !crate::sessions::valid_id(id) || !matches!(stream, "stdout" | "stderr") {
            return Err("Invalid output reference".into());
        }
        if session.is_some_and(|session| !crate::sessions::valid_id(session)) {
            return Err("Invalid output reference".into());
        }
        ownership::resolve(&self.root, self.owner.as_ref(), session, id, stream)
    }
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
