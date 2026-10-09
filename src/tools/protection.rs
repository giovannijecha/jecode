use super::{files, required_string, watch::fingerprint};
use crate::{
    cancel::Cancellation,
    json::{self, Value},
    output::Store,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

mod decisions;
mod directories;
#[cfg(test)]
mod directory_tests;
mod recovery;
mod registration;
mod scope;
#[cfg(test)]
mod scope_tests;
mod storage;
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct Protection {
    files: BTreeMap<PathBuf, Entry>,
    request: usize,
    reported: BTreeMap<PathBuf, Value>,
    pending: Vec<(usize, usize, Value)>,
    reviewed_request: Option<usize>,
    scope_blocked: bool,
    scope_exclusions: BTreeSet<PathBuf>,
    scope_reason: String,
    directories: BTreeMap<PathBuf, directories::Declaration>,
}

#[derive(Clone)]
struct Entry {
    snapshot: String,
    baseline: Value,
    request: usize,
    reason: String,
    require_check: bool,
    released: bool,
    history: Option<usize>,
}

impl Protection {
    pub fn begin_request(&mut self, index: usize) {
        self.request = index;
        self.scope_blocked = false;
        self.scope_exclusions.clear();
        self.scope_reason.clear();
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn refuse_write(&self, root: &Path, path: &str) -> Result<(), String> {
        self.require_registration()?;
        let target = files::writable_path(root, path)?;
        let mut lexical = PathBuf::new();
        if let Ok(relative) = root.join(path).strip_prefix(root) {
            for part in relative.components() {
                match part {
                    Component::Normal(value) => lexical.push(value),
                    Component::ParentDir => {
                        lexical.pop();
                    }
                    _ => {}
                }
            }
        }
        if [target.strip_prefix(root).unwrap(), lexical.as_path()]
            .iter()
            .any(|path| self.files.get(*path).is_some_and(|entry| !entry.released))
        {
            return Err("This file is protected. Use protect action=status then action=restore for incidental changes. Release requires a newer user request explicitly allowing edits; reading or writing cannot approve a violation.".into());
        }
        Ok(())
    }

    pub fn require_registration(&self) -> Result<(), String> {
        if self.pending.is_empty() {
            Ok(())
        } else {
            Err("An interrupted protection registration has no recoverable original version. Use protect status; releasing its incomplete-registration marker requires a newer user request explicitly accepting that uncertainty before mutations or a new baseline".into())
        }
    }

    pub fn execute(
        &mut self,
        root: &Path,
        store: &Store,
        arguments: &Value,
        cancellation: &Cancellation,
    ) -> Value {
        let operation = (|| {
            let action = required_string(arguments, "action")?;
            self.prepare_scope(root, action, arguments)?;
            let paths = match arguments.get("paths") {
                None if action == "status" => Vec::new(),
                Some(Value::Array(paths)) => paths
                    .iter()
                    .map(|path| path.as_str().ok_or("paths must contain project paths"))
                    .collect::<Result<Vec<_>, _>>()?,
                _ => return Err("paths must be an array of project paths".into()),
            };
            match action {
                "record" => self.register(root, store, arguments, &paths, cancellation),
                "status" => Ok(Value::object([] as [(&str, Value); 0])),
                "restore" | "release" => {
                    if paths.len() != 1 {
                        return Err(
                            "restore/release requires exactly one protected file path".into()
                        );
                    }
                    if action == "release" && self.release_pending(paths[0], arguments)? {
                        return Ok(Value::object([("released", Value::Bool(true))]));
                    }
                    if action == "release"
                        && let Some(result) = self.release_directory(root, paths[0], arguments)?
                    {
                        return Ok(result);
                    }
                    let absolute = files::writable_path(root, paths[0])?;
                    let path = absolute.strip_prefix(root).unwrap();
                    let entry = self
                        .files
                        .get_mut(path)
                        .ok_or("File has no recorded protection baseline")?;
                    if entry.released {
                        return Err("File protection has been released".into());
                    }
                    if action == "release" {
                        if self.request <= entry.request {
                            return Err("Protection cannot be released without a newer user request explicitly allowing the change".into());
                        }
                        let reason = required_string(arguments, "reason")?;
                        if reason.trim().is_empty() {
                            return Err("reason must describe the newer user's decision".into());
                        }
                        entry.released = true;
                        entry.reason = reason.into();
                        Ok(Value::object([("released", Value::Bool(true))]))
                    } else {
                        let expected = required_string(arguments, "expected_current")?;
                        let saved = store.protection_path(&entry.snapshot)?;
                        let bytes = storage::restore(
                            root,
                            path,
                            &saved,
                            &entry.baseline,
                            expected,
                            cancellation,
                        )?;
                        Ok(Value::object([
                            ("bytes_written", Value::number(bytes)),
                            ("restored_path", Value::string(path.to_string_lossy())),
                        ]))
                    }
                }
                _ => Err("action must be record, status, restore or release".into()),
            }
        })();
        match operation {
            Ok(mut result) => {
                self.finish_scope_review(root, cancellation);
                if let Value::Object(fields) = &mut result {
                    fields.insert("scope_review".into(), self.scope_view(root, cancellation));
                }
                result
            }
            Err(error) => Value::object([("error", Value::string(error))]),
        }
    }

    pub fn status(&self, root: &Path, store: &Store, cancellation: &Cancellation) -> Vec<Value> {
        let mut result = self
            .files
            .iter()
            .map(|(path, entry)| {
                let mut result = Value::object([
                    ("path", Value::string(path.to_string_lossy())),
                    ("snapshot_id", Value::string(&entry.snapshot)),
                    ("baseline", entry.baseline.clone()),
                    ("registered_request", Value::number(entry.request)),
                    ("reason", Value::string(&entry.reason)),
                    ("require_check", Value::Bool(entry.require_check)),
                    ("scope_review_required", Value::Bool(self.scope_blocked)),
                ]);
                let checked: Result<(&str, Value), String> = if entry.released {
                    Ok(("released", Value::Null))
                } else {
                    (|| {
                        let saved = store.protection_path(&entry.snapshot)?;
                        storage::verify_snapshot(&saved, &entry.baseline, cancellation)?;
                        let current = fingerprint::capture(root, path, cancellation)?;
                        let preserved = current.get("state").and_then(Value::as_str)
                            == Some("present")
                            && storage::same_bytes(&root.join(path), &saved, cancellation)?;
                        if fingerprint::capture(root, path, cancellation)? != current {
                            return Err("Protected file changed during comparison".into());
                        }
                        Ok((if preserved { "preserved" } else { "violated" }, current))
                    })()
                };
                if let Value::Object(fields) = &mut result {
                    match checked {
                        Ok((state, current)) => {
                            fields.insert("state".into(), Value::string(state));
                            if state != "released" {
                                fields.insert(
                                    "expected_current".into(),
                                    Value::string(storage::token(&current)),
                                );
                                fields.insert("current".into(), current);
                            }
                        }
                        Err(error) => {
                            fields.insert("state".into(), Value::string("unknown"));
                            fields.insert("error".into(), Value::string(error));
                        }
                    }
                    if let Some(at) = entry.history {
                        fields.insert(
                            "history_reference".into(),
                            Value::string(format!("history:{at}")),
                        );
                    }
                }
                result
            })
            .collect::<Vec<_>>();
        for (request, at, arguments) in &self.pending {
            result.push(Value::object([
                ("path", Value::string(format!("Incomplete registration at history:{at}"))),
                ("state", Value::string("unknown")), ("registration_incomplete", Value::Bool(true)),
                ("registered_request", Value::number(*request)), ("history_reference", Value::string(format!("history:{at}"))),
                ("declared_paths", arguments.get("paths").cloned().unwrap_or(Value::Null)),
                ("reason", Value::string("Interrupted registration has no recoverable original version. Do not rebase it; release this marker only after a newer user request explicitly accepts the uncertainty")),
            ]));
        }
        result
    }

    pub fn active_paths(&self, root: &Path) -> Vec<PathBuf> {
        self.files
            .iter()
            .filter(|(_, entry)| !entry.released)
            .map(|(path, _)| root.join(path))
            .collect()
    }

    pub fn changes(
        &mut self,
        root: &Path,
        store: &Store,
        cancellation: &Cancellation,
        force: bool,
    ) -> Vec<Value> {
        self.status(root, store, cancellation)
            .into_iter()
            .filter(|value| {
                let path = PathBuf::from(value.get("path").and_then(Value::as_str).unwrap());
                let changed = self.reported.get(&path) != Some(value);
                self.reported.insert(path, value.clone());
                force || changed
            })
            .collect()
    }

    pub fn record(&mut self, at: usize, result: &Value) {
        for value in result
            .get("file_protections")
            .and_then(Value::as_array)
            .unwrap_or(&[])
        {
            if let Some(path) = value.get("path").and_then(Value::as_str)
                && let Some(entry) = self.files.get_mut(Path::new(path))
                && entry.history.is_none()
            {
                entry.history = Some(at);
                // The receipt is already in history; adding its reference is not
                // a filesystem change requiring every baseline to be sent again.
                if let Some(Value::Object(fields)) = self.reported.get_mut(Path::new(path)) {
                    fields.insert(
                        "history_reference".into(),
                        Value::string(format!("history:{at}")),
                    );
                }
            }
        }
    }

    pub fn rebuild(&mut self, messages: &[Value]) -> Result<(), String> {
        self.reset();
        let mut calls = BTreeMap::new();
        for (at, message) in messages.iter().enumerate() {
            if message.get("role").and_then(Value::as_str) == Some("user") {
                self.begin_request(at);
            }
            if let Some(batch) = message.get("tool_calls").and_then(Value::as_array) {
                for call in batch {
                    if let Some(id) = call.get("id").and_then(Value::as_str) {
                        calls.insert(id, call);
                    }
                }
            }
            if message.get("role").and_then(Value::as_str) != Some("tool") {
                continue;
            }
            let Some(result) = message
                .get("content")
                .and_then(Value::as_str)
                .and_then(|text| json::parse(text).ok())
            else {
                continue;
            };
            self.rebuild_directories(message, &result, &calls)?;
            self.rebuild_scope(message, &result, &calls);
            if result.get("file_protections_scope").and_then(Value::as_str) == Some("all")
                && result
                    .get("file_protections")
                    .and_then(Value::as_array)
                    .is_some_and(|entries| {
                        entries.iter().all(|entry| {
                            entry.get("registration_incomplete") != Some(&Value::Bool(true))
                        })
                    })
            {
                self.pending.clear();
            }
            if result.get("outcome").and_then(Value::as_str) == Some("unknown")
                && let Some(call) = message
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .and_then(|id| calls.get(id))
                && call
                    .get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
                    == Some("protect")
                && let Some(arguments) = call
                    .get("function")
                    .and_then(|function| function.get("arguments"))
                    .and_then(Value::as_str)
                    .and_then(|text| json::parse(text).ok())
                && arguments.get("action").and_then(Value::as_str) == Some("record")
            {
                self.pending.push((self.request, at, arguments));
            }
            for value in result
                .get("file_protections")
                .and_then(Value::as_array)
                .unwrap_or(&[])
            {
                if value.get("registration_incomplete") == Some(&Value::Bool(true)) {
                    continue;
                }
                let path = PathBuf::from(required_string(value, "path")?);
                if path.as_os_str().is_empty()
                    || path
                        .components()
                        .any(|part| !matches!(part, Component::Normal(_)))
                {
                    return Err("Invalid saved protection path".into());
                }
                let snapshot = required_string(value, "snapshot_id")?;
                if !crate::sessions::valid_id(snapshot) {
                    return Err("Invalid saved protection snapshot".into());
                }
                let baseline = value
                    .get("baseline")
                    .and_then(fingerprint::version)
                    .ok_or("Invalid saved protection baseline")?;
                if baseline.get("state").and_then(Value::as_str) != Some("present") {
                    return Err("Invalid saved protection baseline state".into());
                }
                let history = self
                    .files
                    .get(&path)
                    .filter(|entry| entry.snapshot == snapshot)
                    .and_then(|entry| entry.history)
                    .unwrap_or(at);
                self.files.insert(
                    path,
                    Entry {
                        snapshot: snapshot.into(),
                        baseline,
                        request: value
                            .get("registered_request")
                            .and_then(Value::as_usize)
                            .ok_or("Invalid saved protection request")?,
                        reason: required_string(value, "reason")?.into(),
                        require_check: value.get("require_check") == Some(&Value::Bool(true)),
                        released: value.get("state").and_then(Value::as_str) == Some("released"),
                        history: Some(history),
                    },
                );
            }
        }
        Ok(())
    }
}

fn expand(root: &Path, paths: &[&str]) -> Result<BTreeSet<PathBuf>, String> {
    let mut pending = paths
        .iter()
        .map(|path| files::existing_path(root, path))
        .collect::<Result<Vec<_>, _>>()?;
    let mut visited = BTreeSet::new();
    let mut files = BTreeSet::new();
    while let Some(path) = pending.pop() {
        if path.is_file() {
            files.insert(path.strip_prefix(root).unwrap().into());
        } else if path.is_dir() && visited.insert(path.clone()) {
            for entry in fs::read_dir(&path).map_err(|error| error.to_string())? {
                let entry = entry.map_err(|error| error.to_string())?;
                pending.push(files::existing_path(root, &entry.path().to_string_lossy())?);
            }
        } else if !path.is_dir() {
            return Err(
                "Protection accepts existing regular files or directories containing them".into(),
            );
        }
    }
    if files.is_empty() {
        return Err("Protection scope contains no existing regular files".into());
    }
    Ok(files)
}
