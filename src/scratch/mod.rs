use crate::json::{self, Value};
use std::fs::{self, Metadata, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

const MARKER: &str = ".jecode-tmp.json";

/// Disposable working files belong to a folder and a resumable session.
pub struct Area {
    path: PathBuf,
    project: PathBuf,
    session: String,
}

#[derive(Debug)]
pub struct Info {
    pub path: String,
    pub files: usize,
    pub directories: usize,
    pub bytes: u64,
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
        // Only these three owned descendants may be created or cleaned here.
        let parents: Vec<_> = self.path.ancestors().take(3).collect();
        for path in parents.into_iter().rev() {
            private_directory(path)?;
        }
        let root = fs::canonicalize(&self.path)
            .map_err(|error| format!("Could not resolve temporary area: {error}"))?;
        let marker = root.join(MARKER);
        if !marker.try_exists().map_err(|error| error.to_string())? {
            if fs::read_dir(&root)
                .map_err(|error| error.to_string())?
                .next()
                .is_some()
            {
                return Err(
                    "Temporary area has no ownership record; existing files were left unchanged"
                        .into(),
                );
            }
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&marker).map_err(|error| {
                format!("Could not establish temporary area ownership: {error}")
            })?;
            let result = file
                .write_all(self.identity().encode().as_bytes())
                .and_then(|()| file.sync_all());
            drop(file);
            if let Err(error) = result {
                let _ = fs::remove_file(&marker);
                return Err(format!("Could not save temporary area ownership: {error}"));
            }
        }
        let metadata = inspect(&marker)?;
        if !metadata.is_file() {
            return Err("Temporary ownership record must be a regular file".into());
        }
        let owner = fs::read_to_string(&marker)
            .map_err(|error| format!("Could not read temporary area ownership: {error}"))?;
        if json::parse(&owner)? != self.identity() {
            return Err(
                "Temporary area belongs to another folder or session; files were left unchanged"
                    .into(),
            );
        }
        Ok(root)
    }

    fn identity(&self) -> Value {
        Value::object([
            ("format", Value::string("jecode.temporary")),
            ("format_version", Value::number(1)),
            ("directory", Value::string(self.project.to_string_lossy())),
            ("session_id", Value::string(&self.session)),
        ])
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
        };
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
                let path = entry.map_err(|error| error.to_string())?.path();
                if path == root.join(MARKER) {
                    continue;
                }
                let metadata = inspect(&path)?;
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
                    return Err(
                        "Temporary area contains a special file; cleanup was not started".into(),
                    );
                }
            }
        }
        Ok(info)
    }

    pub fn clean(&self) -> Result<Info, String> {
        let root = self.ensure()?;
        // Inspect the entire tree before deleting anything. Links are never followed.
        let info = self.inventory(&root)?;
        let entries = fs::read_dir(&root).map_err(|error| error.to_string())?;
        for entry in entries {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path == root.join(MARKER) {
                continue;
            }
            if self.ensure()? != root || !path.starts_with(&root) {
                return Err(
                    "Temporary area changed during cleanup; remaining files were left unchanged"
                        .into(),
                );
            }
            let metadata = inspect(&path)?;
            let result = if metadata.is_dir() {
                fs::remove_dir_all(&path)
            } else if metadata.is_file() {
                fs::remove_file(&path)
            } else {
                return Err("Temporary entry changed; remaining files were left unchanged".into());
            };
            result.map_err(|error| format!(
                "Temporary cleanup did not finish: {error}. Some temporary files may already have been removed; inspect /tmp before continuing."
            ))?;
        }
        Ok(info)
    }
}

fn private_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            if !inspect(path)?.is_dir() {
                return Err("Temporary area path must be a directory".into());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path)
                .map_err(|error| format!("Could not create temporary area: {error}"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                    .map_err(|error| error.to_string())?;
            }
            inspect(path)?;
        }
        Err(error) => return Err(format!("Could not inspect temporary area: {error}")),
    }
    Ok(())
}

fn inspect(path: &Path) -> Result<Metadata, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect temporary path: {error}"))?;
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
    };
    #[cfg(not(windows))]
    let linked = metadata.file_type().is_symlink();
    if linked {
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
