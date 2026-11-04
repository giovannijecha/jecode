//! Write and edit calls. An edit names one file with `path`, or several with
//! `files`: one call instead of a model round trip per file. Each file of a
//! multi-file edit takes the single-file route and applies all or nothing; a
//! failed file never stops the others, and its entry says why.

use super::{Tools, edit, files};
use crate::cancel::Cancellation;
use crate::file_change::Change;
use crate::json::Value;
use crate::permissions::Kind;
use std::fs;
use std::path::{Path, PathBuf};

impl Tools {
    /// One file through the write or edit route. `base` is the root of an
    /// approved file outside the working folder.
    pub(super) fn change_file(
        &self,
        name: &str,
        arguments: &Value,
        base: Option<&Path>,
        cancellation: &Cancellation,
    ) -> Result<(Value, Option<Change>), String> {
        let (root, arguments) = self.file_arguments(arguments, name == "write")?;
        let (root, arguments) = match base {
            Some(base) => (base.to_path_buf(), self.rebase(base, &arguments)),
            None => (root, arguments),
        };
        // Session temporary files are outside the workspace scope.
        let checkpoints = self.checkpoints.as_deref().filter(|_| root == self.root);
        let result = if name == "write" {
            files::write_with_change(&root, &arguments, checkpoints, cancellation)
        } else {
            files::edit_with_change(&root, &arguments, checkpoints, cancellation)
        };
        result.map(|(mut value, change)| {
            if self.checkpoints.is_some()
                && root != self.root
                && value.get("bytes_written").is_some()
                && let Value::Object(fields) = &mut value
            {
                fields.insert(
                    "checkpoint".into(),
                    Value::object([
                        ("status", Value::string("none")),
                        ("saved_paths", Value::number(0)),
                        (
                            "reason",
                            Value::string(if base.is_some() {
                                "the file is outside the working folder"
                            } else {
                                "session temporary storage is outside the workspace"
                            }),
                        ),
                    ]),
                );
            }
            (value, change)
        })
    }

    /// Each file in call order, each with its own result and checkpoint.
    /// `outside` is set when the call's approved access covered files outside
    /// the working folder; without it such a file fails alone.
    pub(super) fn edit_files(
        &self,
        arguments: &Value,
        outside: bool,
        cancellation: &Cancellation,
    ) -> Result<(Value, Option<Change>), String> {
        let files = edit::files(arguments)?.unwrap_or_default();
        self.distinct(&files)?;
        let mut results = Vec::with_capacity(files.len());
        let mut changes = Vec::new();
        for (path, entry) in files {
            let base = outside
                .then(|| self.file_access(Kind::Edits, path, false))
                .flatten()
                .filter(|access| access.outside)
                .and_then(|access| access.base);
            let (result, change) = self
                .change_file("edit", entry, base.as_deref(), cancellation)
                .unwrap_or_else(|error| (Value::object([("error", Value::string(error))]), None));
            if let Some(change) = change {
                changes.push((path.to_owned(), change));
            }
            results.push(named(path, result));
        }
        Ok((
            Value::object([("files", Value::Array(results))]),
            Change::several(changes),
        ))
    }

    /// Two spellings of one file would edit it from two snapshots, so the
    /// call is refused before any file changes.
    fn distinct(&self, files: &[(&str, &Value)]) -> Result<(), String> {
        let mut seen: Vec<(PathBuf, &str)> = Vec::with_capacity(files.len());
        for &(path, _) in files {
            let Ok(resolved) = fs::canonicalize(self.root.join(path)) else {
                continue;
            };
            if let Some((_, first)) = seen.iter().find(|(file, _)| *file == resolved) {
                return Err(format!(
                    "files names one file twice, as {first} and {path}; put all of its replacements in one entry"
                ));
            }
            seen.push((resolved, path));
        }
        Ok(())
    }
}

/// A file's result under the path the call gave.
fn named(path: &str, result: Value) -> Value {
    let Value::Object(mut fields) = result else {
        return result;
    };
    fields.insert("path".into(), Value::string(path));
    Value::Object(fields)
}
