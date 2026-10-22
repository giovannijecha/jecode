//! Observable filesystem state. Pack offsets are irrelevant to live identity.
use super::files::{self, Mode};
use super::hash::{self, Digest};
use crate::cancel::Cancellation;
use std::fs;
use std::io::Read;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Absent,
    File {
        offset: u64,
        length: u64,
        digest: Digest,
        mode: Mode,
    },
    Directory {
        mode: Mode,
    },
    Link {
        target: String,
        directory: bool,
    },
    Unreadable {
        reason: String,
    },
}

impl State {
    pub fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::File {
                    length: al,
                    digest: ad,
                    mode: am,
                    ..
                },
                Self::File {
                    length: bl,
                    digest: bd,
                    mode: bm,
                    ..
                },
            ) => al == bl && ad == bd && am == bm,
            _ => self == other,
        }
    }

    pub fn recoverable(&self) -> bool {
        !matches!(self, Self::Unreadable { .. })
    }
    pub fn exists(&self) -> bool {
        !matches!(self, Self::Absent)
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::File { .. } => "file",
            Self::Directory { .. } => "directory",
            Self::Link { .. } => "link",
            Self::Unreadable { .. } => "unreadable",
        }
    }
}

pub fn observe(root: &Path, path: &str, cancel: &Cancellation) -> Result<State, String> {
    if cancel.requested() {
        return Err("Recovery observation was cancelled".into());
    }
    files::confined_parent(root, path)?;
    let absolute = super::paths::absolute(root, path);
    let metadata = match fs::symlink_metadata(&absolute) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(State::Absent),
        Err(error) => return Err(format!("Could not inspect {path}: {error}")),
    };
    let kind = metadata.file_type();
    if kind.is_symlink() {
        let target = fs::read_link(&absolute)
            .map_err(|error| format!("Could not read link {path}: {error}"))?;
        #[cfg(windows)]
        let directory = {
            use std::os::windows::fs::FileTypeExt;
            kind.is_symlink_dir()
        };
        #[cfg(not(windows))]
        let directory = false;
        return Ok(match target.to_str() {
            Some(target) => State::Link {
                target: target.into(),
                directory,
            },
            None => State::Unreadable {
                reason: "Link target has an unsupported name".into(),
            },
        });
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Ok(State::Unreadable {
                reason: "Unsupported reparse point".into(),
            });
        }
    }
    if kind.is_dir() {
        return Ok(State::Directory {
            mode: Mode::of(&metadata),
        });
    }
    if !kind.is_file() {
        return Ok(State::Unreadable {
            reason: "Unsupported filesystem entry".into(),
        });
    }
    let mut file =
        fs::File::open(&absolute).map_err(|error| format!("Could not open {path}: {error}"))?;
    let source = file
        .metadata()
        .map_err(|error| format!("Could not inspect open {path}: {error}"))?;
    if !source.is_file() {
        return Err(format!("{path} changed type while opening"));
    }
    let length = source.len();
    let mut digest = hash::Hasher::new(length);
    let mut remaining = length;
    let mut buffer = [0u8; 64 * 1024];
    while remaining > 0 {
        if cancel.requested() {
            return Err("Recovery observation was cancelled".into());
        }
        let wanted = remaining.min(buffer.len() as u64) as usize;
        let count = file
            .read(&mut buffer[..wanted])
            .map_err(|error| format!("Could not read {path}: {error}"))?;
        if count == 0 {
            return Err(format!("{path} changed while reading"));
        }
        digest.update(&buffer[..count]);
        remaining -= count as u64;
    }
    let later = fs::symlink_metadata(&absolute)
        .map_err(|error| format!("Could not recheck {path}: {error}"))?;
    if later.len() != length || !later.is_file() || Mode::of(&later) != Mode::of(&source) {
        return Err(format!("{path} changed while reading"));
    }
    Ok(State::File {
        offset: 0,
        length,
        digest: digest.finish(),
        mode: Mode::of(&source),
    })
}
