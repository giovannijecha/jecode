//! Working-directory resolution is separate from the selected file-access profile.
use super::{Error, Opened, Workspace, path, platform};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Access {
    #[default]
    Workspace,
    Local,
}
impl Access {
    pub fn name(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Local => "local",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "workspace" => Some(Self::Workspace),
            "local" => Some(Self::Local),
            _ => None,
        }
    }
}

pub(super) struct Location {
    pub display: String,
    relative: Option<String>,
    absolute: PathBuf,
}
impl Workspace {
    pub fn with_access(mut self, access: Access) -> Self {
        self.access = access;
        self
    }
    pub fn access(&self) -> Access {
        self.access
    }
    pub fn instructions(&self) -> String {
        let directory = crate::json::encode(
            &crate::json::Value::String(display(&self.display).unwrap_or_default()),
            65536,
        )
        .unwrap_or_default();
        let policy = match self.access {
            Access::Workspace => {
                "Only forward-slash paths relative to this workspace are available; parent traversal and absolute paths are not allowed."
            }
            Access::Local => {
                "Paths may be absolute or relative to this working directory, including parent directories. Ordinary local files outside it can be read directly. Each file change and command still requires approval. Dot paths and known credential names are excluded. Links and network paths are not supported. Generated directories are omitted from discovery; known files can be addressed directly."
            }
        };
        format!(
            "Working directory (JSON string): {directory}. File access: {}. {policy}",
            self.access.name()
        )
    }
    pub(super) fn resolve(&self, input: &str) -> Result<Location, Error> {
        if self.access == Access::Workspace {
            let relative = super::relative(input)?;
            return Ok(Location {
                absolute: self.display.join(&relative),
                display: relative.clone(),
                relative: Some(relative),
            });
        }
        path::input(input)?;
        let base = absolute(&self.display)?;
        let requested = Path::new(input);
        let target = absolute(&if requested.is_absolute() {
            requested.to_owned()
        } else {
            base.join(requested)
        })?;
        let relative = target
            .strip_prefix(&base)
            .ok()
            .map(|p| {
                if p.as_os_str().is_empty() {
                    Ok(".".into())
                } else {
                    display(p)
                }
            })
            .transpose()?;
        Ok(Location {
            display: display(&target)?,
            relative,
            absolute: target,
        })
    }
    pub(super) fn open_location(&self, path: &Location, directory: bool) -> Result<Opened, Error> {
        match &path.relative {
            Some(relative) => platform::open(&self.root, relative, directory),
            None => platform::absolute(&path.absolute, directory),
        }
        .map_err(|_| Error::Unavailable)
    }
}

fn absolute(path: &Path) -> Result<PathBuf, Error> {
    if !path.is_absolute() {
        return Err(Error::Path);
    }
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            #[cfg(windows)]
            Component::Prefix(prefix) => match prefix.kind() {
                std::path::Prefix::Disk(drive) | std::path::Prefix::VerbatimDisk(drive) => {
                    result.push(format!("{}:/", char::from(drive).to_ascii_uppercase()));
                }
                _ => return Err(Error::Path),
            },
            #[cfg(not(windows))]
            Component::Prefix(_) => return Err(Error::Path),
            Component::RootDir => result.push(std::path::MAIN_SEPARATOR_STR),
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() {
                    return Err(Error::Path);
                }
            }
            Component::Normal(name) => {
                path::name(name.to_str().ok_or(Error::Path)?, false)?;
                result.push(name);
            }
        }
    }
    Ok(result)
}
pub(super) fn display(path: &Path) -> Result<String, Error> {
    let text = path.to_str().ok_or(Error::Path)?;
    #[cfg(windows)]
    let text = text
        .strip_prefix(r"\\?\")
        .unwrap_or(text)
        .replace('\\', "/");
    Ok(text.to_owned())
}
