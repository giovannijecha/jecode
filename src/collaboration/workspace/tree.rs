use super::files;
use crate::disk;
use crate::json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Omitted entries named in a report; the count stays exact beyond it.
const SAMPLE: usize = 50;

/// Entries a copy or comparison leaves out, with a bounded named sample.
#[derive(Clone, Debug, Default)]
pub(super) struct Omitted {
    pub(super) count: usize,
    pub(super) sample: Vec<(String, String)>,
    /// Every name omitted by this walk; not saved.
    pub(super) names: BTreeSet<String>,
}

impl Omitted {
    pub(super) fn add(&mut self, name: impl Into<String>, reason: impl Into<String>) {
        let name = name.into();
        self.count += 1;
        if self.sample.len() < SAMPLE {
            self.sample.push((name.clone(), reason.into()));
        }
        self.names.insert(name);
    }

    pub(super) fn value(&self) -> Value {
        Value::object([
            ("count", Value::number(self.count)),
            (
                "entries",
                Value::Array(
                    self.sample
                        .iter()
                        .map(|(path, reason)| {
                            Value::object([
                                ("path", Value::string(path)),
                                ("reason", Value::string(reason)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }

    pub(super) fn from_value(value: Option<&Value>) -> Self {
        let Some(value) = value else {
            return Self::default();
        };
        let sample = value
            .get("entries")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        Some((
                            entry.get("path")?.as_str()?.to_owned(),
                            entry.get("reason")?.as_str()?.to_owned(),
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Self {
            count: value
                .get("count")
                .and_then(Value::as_usize)
                .unwrap_or(sample.len()),
            names: sample.iter().map(|(name, _)| name.clone()).collect(),
            sample,
        }
    }
}

pub(super) enum Kind {
    Directory,
    File,
    Link,
}

pub(super) struct Item {
    pub(super) name: String,
    pub(super) path: PathBuf,
    pub(super) kind: Kind,
    pub(super) metadata: fs::Metadata,
}

/// Visit every ordinary project directory, file and symbolic link, each
/// directory before its contents, without following links. Jecode/Git
/// metadata is excluded at every depth, and root `target`/`.tmp` entries other
/// than regular files are omitted silently. Entries a copy cannot represent, such as
/// sockets or names that are not portable, are omitted and reported, as are
/// directories that cannot be listed. An entry removed during the walk is
/// treated as absent.
pub(super) fn walk(
    root: &Path,
    mut visit: impl FnMut(Item) -> Result<(), String>,
) -> Result<Omitted, String> {
    let metadata = fs::symlink_metadata(root).map_err(|error| error.to_string())?;
    if !disk::ordinary_directory(&metadata) {
        return Err("Workspace inventory root is not an ordinary directory".into());
    }
    let mut omitted = Omitted::default();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((directory, prefix)) = pending.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if prefix.is_empty() => return Err(error.to_string()),
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                omitted.add(&prefix, format!("Could not list directory: {error}"));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    omitted.add(&prefix, format!("Could not list directory: {error}"));
                    break;
                }
            };
            let component = entry.file_name();
            let Some(component) = component.to_str() else {
                omitted.add(
                    join(&prefix, &component.to_string_lossy()),
                    "Name is not valid UTF-8",
                );
                continue;
            };
            if matches!(component, ".git" | ".jecode") {
                continue;
            }
            let name = join(&prefix, component);
            if let Err(error) = files::validate_name(&name) {
                omitted.add(&name, format!("{error}; the copy cannot represent it"));
                continue;
            }
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => {
                    omitted.add(&name, format!("Could not inspect: {error}"));
                    continue;
                }
            };
            // Root build and scratch trees are left out whether they are
            // directories or links; ordinary files with those names stay.
            if prefix.is_empty() && matches!(component, "target" | ".tmp") && !metadata.is_file() {
                continue;
            }
            let kind = if metadata.file_type().is_symlink() {
                Kind::Link
            } else if disk::ordinary_directory(&metadata) {
                Kind::Directory
            } else if metadata.is_file() {
                // On Windows, std reports only name-surrogate reparse points
                // as links; others, such as cloud-file placeholders, are content.
                Kind::File
            } else if metadata.is_dir() {
                omitted.add(&name, "Unsupported reparse point");
                continue;
            } else {
                omitted.add(&name, "Socket, pipe or device; not copied");
                continue;
            };
            let path = entry.path();
            if matches!(kind, Kind::Directory) {
                pending.push((path.clone(), name.clone()));
            }
            visit(Item {
                name,
                path,
                kind,
                metadata,
            })?;
        }
    }
    Ok(omitted)
}

/// The names `walk` visits, for comparing a copy.
pub(super) fn inventory(root: &Path) -> Result<(BTreeSet<String>, Omitted), String> {
    let mut names = BTreeSet::new();
    let omitted = walk(root, |item| {
        names.insert(item.name);
        Ok(())
    })?;
    Ok((names, omitted))
}

fn join(prefix: &str, component: &str) -> String {
    if prefix.is_empty() {
        component.to_owned()
    } else {
        format!("{prefix}/{component}")
    }
}
