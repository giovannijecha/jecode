use crate::disk::owned::{self, Marker};
use crate::json::Value;
use std::fs::{self, Metadata};
use std::path::{Component, Path, PathBuf};

const MARKER: &str = ".jecode-tmp.json";
const OWNERSHIP: Marker = Marker {
    name: MARKER,
    format: "jecode.temporary",
    label: "Temporary",
    max_bytes: None,
};

/// Disposable working files belong to a folder and a resumable session.
pub struct Area {
    path: PathBuf,
    project: PathBuf,
    session: String,
    establishment: std::sync::Mutex<()>,
}

#[derive(Debug)]
pub struct Info {
    pub path: String,
    pub files: usize,
    pub directories: usize,
    pub bytes: u64,
    pub skipped: usize,
}

mod removal;
pub(crate) use removal::Removal;

impl Area {
    pub fn new(path: PathBuf, project: PathBuf, session: &str) -> Result<Self, String> {
        if !path.is_absolute() || !crate::sessions::valid_id(session) {
            return Err("Invalid temporary area identity".into());
        }
        Ok(Self {
            path,
            project,
            session: session.into(),
            establishment: std::sync::Mutex::new(()),
        })
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn relative_path(&self, value: &str) -> Option<String> {
        let path = environment_path(Path::new(value));
        let base = environment_path(&self.path);
        #[cfg(windows)]
        {
            let mut path = path.to_string_lossy().into_owned();
            let bytes = path.as_bytes();
            if bytes.len() >= 3
                && bytes[0] == b'/'
                && bytes[1].is_ascii_alphabetic()
                && bytes[2] == b'/'
            {
                path = format!("{}:{}", char::from(bytes[1]), &path[2..]);
            }
            let base = base.to_string_lossy();
            let prefix = path.get(..base.len())?;
            if prefix.eq_ignore_ascii_case(&base) {
                return path[base.len()..].strip_prefix('/').map(str::to_string);
            }
            None
        }
        #[cfg(not(windows))]
        {
            path.strip_prefix(base)
                .ok()
                .map(|relative| relative.to_string_lossy().into_owned())
        }
    }

    pub fn instructions(&self) -> String {
        let state = Value::object([
            (
                "path",
                Value::string(environment_path(&self.path).to_string_lossy()),
            ),
            ("file_path_format", Value::string("tmp:relative/path")),
            ("absolute_file_paths", Value::Bool(true)),
            ("write_creates_parents", Value::Bool(true)),
            (
                "bash_environment",
                Value::Array(
                    ["JECODE_TMP", "TMPDIR", "TEMP", "TMP"]
                        .map(Value::string)
                        .into(),
                ),
            ),
            ("lifetime", Value::string("session_including_resume")),
            (
                "cleanup",
                Value::string("explicit_tmp_clean_or_session_deletion"),
            ),
            (
                "saved_outputs_and_history",
                Value::string("separate_storage"),
            ),
        ]);
        format!("Session temporary area:\n{}", state.encode())
    }

    pub fn ensure(&self) -> Result<PathBuf, String> {
        // Parallel acquisitions share this area. Publish its ownership record
        // completely before another caller validates it or creates artifacts.
        let _establishment = self.establishment.lock().unwrap();
        // Only these three owned descendants may be created or cleaned here.
        let parents: Vec<_> = self.path.ancestors().take(3).collect();
        for path in parents.into_iter().rev() {
            private_directory(path)?;
        }
        let root = fs::canonicalize(&self.path)
            .map_err(|error| format!("Could not resolve temporary area: {error}"))?;
        self.owned_directory(root.clone()).ensure()?;
        Ok(root)
    }

    fn identity(&self) -> Value {
        OWNERSHIP.identity(&self.project, &self.session)
    }

    fn owned_directory(&self, path: PathBuf) -> owned::Directory {
        let boundary = path.ancestors().take(4).last().unwrap().to_path_buf();
        owned::Directory::new(path, boundary, OWNERSHIP, self.identity())
    }

    pub fn file(&self, alias: &str, create_parents: bool) -> Result<(PathBuf, PathBuf), String> {
        let relative = PathBuf::from(alias.replace('\\', "/"));
        if relative.as_os_str().is_empty()
            || relative.components().any(|part| match part {
                Component::Normal(name) => {
                    name.to_string_lossy().contains(':')
                        || name.to_string_lossy().eq_ignore_ascii_case(MARKER)
                }
                _ => true,
            })
        {
            return Err(
                "tmp: must name a relative file without traversal or reserved metadata paths"
                    .into(),
            );
        }
        let root = self.ensure()?;
        let parts: Vec<_> = relative.components().collect();
        let mut path = root.clone();
        for (index, part) in parts.iter().enumerate() {
            path.push(part.as_os_str());
            let parent = index + 1 < parts.len();
            if parent && create_parents {
                private_directory(&path)?;
            } else {
                match inspect(&path) {
                    Ok(metadata) if parent && !metadata.is_dir() => {
                        return Err("Temporary file parent must be a directory".into());
                    }
                    Ok(_) => {}
                    Err(_) if create_parents && !parent && !path.try_exists().unwrap_or(true) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Ok((root, path))
    }

    pub fn info(&self) -> Result<Info, String> {
        let root = self.ensure()?;
        self.inventory(&root)
    }

    fn inventory(&self, root: &Path) -> Result<Info, String> {
        let mut info = Info {
            path: environment_path(&self.path).to_string_lossy().into_owned(),
            files: 0,
            directories: 0,
            bytes: 0,
            skipped: 0,
        };
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            let entries = match fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.to_string()),
            };
            for entry in entries {
                let path = entry.map_err(|error| error.to_string())?.path();
                if path == root.join(MARKER) {
                    continue;
                }
                let metadata = match fs::symlink_metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        continue;
                    }
                    Err(error) => return Err(error.to_string()),
                };
                if crate::disk::linked(&metadata) {
                    info.skipped += 1;
                    continue;
                }
                if metadata.is_dir() {
                    info.directories += 1;
                    pending.push(path);
                } else if metadata.is_file() {
                    info.files += 1;
                    info.bytes = info
                        .bytes
                        .checked_add(metadata.len())
                        .ok_or("Temporary byte count overflow")?;
                } else {
                    info.skipped += 1;
                }
            }
        }
        Ok(info)
    }

    pub fn clean(&self) -> Result<Info, String> {
        let root = self.ensure()?;
        let removal = self
            .owned_directory(root)
            .prepare_removal()?
            .ok_or("Temporary area disappeared before cleanup")?;
        let (files, directories, bytes) = removal.contents()?;
        removal.clean()?;
        Ok(Info {
            path: environment_path(&self.path).to_string_lossy().into_owned(),
            files,
            directories,
            bytes,
            skipped: 0,
        })
    }
}

fn private_directory(path: &Path) -> Result<(), String> {
    crate::disk::ensure_private_directory(path)
        .map(|_| ())
        .map_err(|error| format!("Could not prepare temporary area: {error}"))
}

fn inspect(path: &Path) -> Result<Metadata, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect temporary path: {error}"))?;
    if crate::disk::linked(&metadata) {
        return Err("Temporary area contains a link or junction; it was left unchanged".into());
    }
    Ok(metadata)
}

/// Win32 extended paths are unsuitable for Git Bash and many child programs.
pub fn environment_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        let text = if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{unc}")
        } else {
            text.strip_prefix(r"\\?\").unwrap_or(&text).into()
        };
        PathBuf::from(text.replace('\\', "/"))
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod removal_tests;
#[cfg(test)]
mod tests;
