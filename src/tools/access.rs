//! What a call would access, for the conversation's permission level. Paths
//! resolve as the tools resolve them; a path the tool will reject anyway is
//! treated as inside, so the tool reports its own error.

use super::{Tools, attachment, exec, show};
use crate::json::{self, Value};
use crate::permissions::{Access, Kind, plain};
use std::fs;
use std::path::{Path, PathBuf};

impl Tools {
    /// `None` for calls the levels do not govern: ordinary web retrieval,
    /// process observation, read-only agent actions, the plan and questions.
    /// Browser control has a separate Full-only gate in `Agent::permit`.
    pub fn access(&self, name: &str, arguments: &str) -> Option<Access> {
        let arguments = json::parse(arguments).ok()?;
        let text = |key| arguments.get(key).and_then(Value::as_str);
        match name {
            "read" => self.file_access(Kind::Reads, text("path")?, false),
            "list" | "search" => self.file_access(Kind::Reads, text("path").unwrap_or("."), true),
            "write" | "edit" => self.file_access(Kind::Edits, text("path")?, false),
            "checkpoint" if text("action") == Some("restore") => Some(self.inside(
                Kind::Edits,
                format!("checkpoint {}", text("id").unwrap_or("restore")),
            )),
            "agent" if text("action") == Some("integrate") => {
                Some(self.inside(Kind::Edits, "collaborator integration".into()))
            }
            "agent" if text("action") == Some("create") && creates_isolated(&arguments) => {
                Some(self.inside(Kind::Edits, "isolated collaborator worktree".into()))
            }
            "agent" if text("action") == Some("close") => {
                Some(self.inside(Kind::Edits, "collaborator worktree removal".into()))
            }
            "process" if text("action") == Some("write") => Some(self.inside(
                Kind::Runs,
                format!("process {} input", text("process_id").unwrap_or("unknown")),
            )),
            "web" if matches!(text("action"), Some("fetch" | "download")) => {
                let method = text("method").unwrap_or(if arguments.get("body").is_some() {
                    "POST"
                } else {
                    "GET"
                });
                (!matches!(method, "GET" | "HEAD") || arguments.get("body").is_some())
                    .then(|| self.inside(Kind::Edits, format!("HTTP {method} request")))
            }
            "bash" => Some(self.inside(Kind::Runs, text("command")?.to_owned())),
            "exec" => {
                let cwd = exec::working_directory(&self.root, &arguments).ok()?;
                let command = std::iter::once(text("executable")?)
                    .chain(
                        arguments
                            .get("args")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str),
                    )
                    .collect::<Vec<_>>()
                    .join(" ");
                let outside = !cwd.starts_with(&self.root);
                let identity = outside
                    .then(|| exact_command_identity(&arguments))
                    .transpose()
                    .ok()?;
                Some(Access {
                    kind: Kind::Runs,
                    target: command,
                    place: outside.then(|| plain(&cwd)),
                    outside,
                    base: None,
                    identity,
                })
            }
            _ => None,
        }
    }

    fn inside(&self, kind: Kind, target: String) -> Access {
        Access {
            kind,
            target,
            place: None,
            outside: false,
            base: None,
            identity: None,
        }
    }

    fn file_access(&self, kind: Kind, path: &str, directory: bool) -> Option<Access> {
        // Jecode's own areas: references, session history and temporary
        // storage. Their tools check them; edits there change no project.
        let temporary = path.starts_with("tmp:")
            || self
                .temporary
                .as_ref()
                .is_some_and(|area| area.relative_path(path).is_some());
        if temporary
            || ["history:", "skill:"].iter().any(|p| path.starts_with(p))
            || attachment::is_reference(path)
            || crate::output::is_reference(path)
            || show::is_reference(path)
        {
            return (kind == Kind::Reads).then(|| self.inside(kind, path.to_owned()));
        }
        let Some(resolved) = resolve(&self.root.join(path)) else {
            return Some(self.inside(kind, path.to_owned()));
        };
        let outside = !resolved.starts_with(&self.root);
        let base = outside.then(|| {
            if directory && resolved.is_dir() {
                resolved.clone()
            } else {
                resolved
                    .parent()
                    .map_or_else(|| resolved.clone(), Path::to_path_buf)
            }
        });
        Some(Access {
            kind,
            target: plain(&resolved),
            place: None,
            outside,
            base,
            identity: None,
        })
    }
}

fn exact_command_identity(arguments: &Value) -> Result<String, String> {
    let value = Value::object([
        (
            "executable",
            arguments.get("executable").cloned().unwrap_or(Value::Null),
        ),
        (
            "args",
            arguments.get("args").cloned().unwrap_or(Value::Null),
        ),
        ("env", arguments.get("env").cloned().unwrap_or(Value::Null)),
        (
            "stdin",
            arguments.get("stdin").cloned().unwrap_or(Value::Null),
        ),
        ("tty", arguments.get("tty").cloned().unwrap_or(Value::Null)),
    ]);
    crate::permissions::identity::sha256_hex(value.encode().as_bytes())
}

fn creates_isolated(arguments: &Value) -> bool {
    arguments
        .get("tasks")
        .and_then(Value::as_array)
        .is_some_and(|tasks| {
            tasks
                .iter()
                .any(|task| task.get("workspace").and_then(Value::as_str) == Some("isolated"))
        })
}

/// The canonical path, or a new file's canonical parent joined with its name.
fn resolve(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path).ok().or_else(|| {
        let parent = fs::canonicalize(path.parent()?).ok()?;
        Some(parent.join(path.file_name()?))
    })
}

#[cfg(test)]
mod tests;
