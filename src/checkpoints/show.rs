//! Recorded content addressed as `checkpoint:cp-N:path`, read without
//! changing the workspace.
use super::index::{parse_reference, reference};
use super::record::State;
use super::restore::normalize;
use super::workspace::Workspace;

pub const PREFIX: &str = "checkpoint:";

pub enum Content {
    Text(String),
    Binary(usize),
    Directory(Vec<String>),
    Link(String),
}

pub fn is_reference(path: &str) -> bool {
    path.starts_with(PREFIX)
}

/// The state of a path at a checkpoint.
pub fn content(workspace: &Workspace, path: &str) -> Result<Content, String> {
    let rest = path
        .strip_prefix(PREFIX)
        .ok_or("Not a checkpoint reference")?;
    let (target, relative) = rest
        .split_once(':')
        .ok_or("Use checkpoint:cp-<number>:<workspace-relative path>")?;
    let target = parse_reference(target)?;
    let relative = normalize(relative)?;
    let mut store = workspace.store();
    store.refresh()?;
    let index = &store.index;
    if !index.retained(target) || target > index.latest() {
        return Err(format!(
            "{} is not available; retained history runs from {} to {}",
            reference(target),
            reference(index.horizon),
            reference(index.latest())
        ));
    }
    match index.state_at(&relative, target) {
        State::File { object, .. } => {
            let bytes = store.bytes(object)?;
            match std::str::from_utf8(&bytes) {
                Ok(text) if !text.contains('\0') => Ok(Content::Text(text.into())),
                _ => Ok(Content::Binary(bytes.len())),
            }
        }
        State::Directory { .. } => {
            let prefix = format!("{relative}/");
            let children = index
                .existing_at(target)
                .filter_map(|version| {
                    let child = version.path.strip_prefix(&prefix)?;
                    let directory = matches!(version.state, State::Directory { .. });
                    (!child.contains('/'))
                        .then(|| format!("{child}{}", if directory { "/" } else { "" }))
                })
                .collect();
            Ok(Content::Directory(children))
        }
        State::Link { target } => Ok(Content::Link(target)),
        State::Absent => Err(format!("{relative} did not exist at {}", reference(target))),
        other => Err(format!(
            "{relative} was not captured at {}: {}",
            reference(target),
            super::actions::describe(&other)
        )),
    }
}
