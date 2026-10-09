use crate::{
    cancel::Cancellation,
    json::{self, Value},
};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

pub(super) mod fingerprint;

#[derive(Default)]
pub(super) struct Watch {
    files: BTreeMap<PathBuf, Observed>,
}

struct Observed {
    first: Value,
    current: Value,
}

#[derive(Default)]
pub(super) struct Report {
    observations: Vec<Value>,
    changes: Vec<Value>,
    errors: Vec<Value>,
}

impl Watch {
    pub fn reset(&mut self) {
        self.files.clear();
    }

    pub fn observe(
        &mut self,
        root: &Path,
        path: &Path,
        source: &str,
        cancellation: &Cancellation,
        report: &mut Report,
    ) {
        let Ok(relative) = path.strip_prefix(root) else {
            return;
        };
        if relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        {
            return;
        }
        match fingerprint::capture(root, relative, cancellation) {
            Ok(version) => {
                if let Some(previous) = self.files.get_mut(relative) {
                    if fingerprint::version(&previous.current).as_ref() != Some(&version) {
                        report.changes.push(Value::object([
                            ("path", Value::string(relative.to_string_lossy())),
                            ("source", Value::string(source)),
                            ("before", previous.current.clone()),
                            ("after", version.clone()),
                            ("first_observed", previous.first.clone()),
                            (
                                "restored_to_first_observed",
                                Value::Bool(
                                    fingerprint::version(&previous.first).as_ref()
                                        == Some(&version),
                                ),
                            ),
                        ]));
                        previous.current = version.clone();
                    }
                } else {
                    self.files.insert(
                        relative.into(),
                        Observed {
                            first: version.clone(),
                            current: version.clone(),
                        },
                    );
                }
                let mut observation = version;
                if let Value::Object(fields) = &mut observation {
                    fields.insert("path".into(), Value::string(relative.to_string_lossy()));
                }
                report.observations.push(observation);
            }
            Err(error) => report.errors.push(Value::object([
                ("path", Value::string(relative.to_string_lossy())),
                ("error", Value::string(error)),
                ("source", Value::string(source)),
            ])),
        }
    }

    pub fn scan(
        &mut self,
        root: &Path,
        source: &str,
        cancellation: &Cancellation,
        report: &mut Report,
    ) {
        let paths = self
            .files
            .keys()
            .map(|path| root.join(path))
            .collect::<Vec<_>>();
        // Persist only initial observations and changes, not every unchanged fingerprint.
        let observations = report.observations.len();
        for path in paths {
            self.observe(root, &path, source, cancellation, report);
        }
        report.observations.truncate(observations);
    }

    pub fn attach(&self, result: &mut Value, report: Report) {
        let Value::Object(fields) = result else {
            return;
        };
        if !report.observations.is_empty() {
            fields.insert(
                "file_observations".into(),
                Value::Array(report.observations),
            );
        }
        fields.insert("file_changes".into(), Value::Array(report.changes));
        fields.insert("file_tracking".into(), Value::object([
            ("scope", Value::string("project files observed through read/write/edit or bash watch; no directory scan")),
            ("watched_files", Value::number(self.files.len())),
            ("status", Value::string(if report.errors.is_empty() { "complete" } else { "incomplete" })),
            ("errors", Value::Array(report.errors)),
        ]));
    }

    pub fn record(&mut self, at: usize, result: &Value) {
        for observation in result
            .get("file_observations")
            .and_then(Value::as_array)
            .unwrap_or(&[])
        {
            self.remember(at, observation.get("path"), observation);
        }
        for change in result
            .get("file_changes")
            .and_then(Value::as_array)
            .unwrap_or(&[])
        {
            if let Some(after) = change.get("after") {
                self.remember(at, change.get("path"), after);
            }
        }
    }

    fn remember(&mut self, at: usize, path: Option<&Value>, value: &Value) {
        let Some(path) = path.and_then(Value::as_str).map(PathBuf::from) else {
            return;
        };
        if path.as_os_str().is_empty()
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return;
        }
        let Some(mut version) = fingerprint::version(value) else {
            return;
        };
        if let Value::Object(fields) = &mut version {
            fields.insert(
                "history_reference".into(),
                Value::string(format!("history:{at}")),
            );
        }
        if let Some(entry) = self.files.get_mut(&path) {
            // Live observations already adopted the new version; now attach its durable reference.
            if entry.first.get("history_reference").is_none() {
                let first = fingerprint::version(&entry.first);
                if first == fingerprint::version(&version) {
                    entry.first = version.clone();
                }
            }
            entry.current = version;
        } else {
            self.files.insert(
                path,
                Observed {
                    first: version.clone(),
                    current: version,
                },
            );
        }
    }

    pub fn rebuild(&mut self, messages: &[Value]) {
        self.reset();
        for (at, message) in messages.iter().enumerate() {
            if message.get("role").and_then(Value::as_str) == Some("tool")
                && let Some(result) = message
                    .get("content")
                    .and_then(Value::as_str)
                    .and_then(|text| json::parse(text).ok())
            {
                self.record(at, &result);
            }
        }
    }
}

#[cfg(test)]
mod tests;
