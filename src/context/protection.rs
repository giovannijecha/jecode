use crate::json::Value;

#[derive(Clone, Debug, Default)]
pub(super) struct Facts {
    pub entries: Vec<Value>,
}

impl Facts {
    pub fn record(&mut self, at: usize, result: &Value) {
        let old_entries = self.entries.clone();
        if result.get("file_protections_scope").and_then(Value::as_str) == Some("all") {
            self.entries.clear();
        }
        for entry in result
            .get("file_protections")
            .and_then(Value::as_array)
            .unwrap_or(&[])
        {
            let mut entry = entry.clone();
            if let Value::Object(fields) = &mut entry {
                let old = old_entries.iter().find(|old| {
                    old.get("path") == fields.get("path")
                        && old.get("snapshot_id") == fields.get("snapshot_id")
                });
                let reference = old
                    .and_then(|old| old.get("history_reference"))
                    .cloned()
                    .unwrap_or_else(|| Value::string(format!("history:{at}")));
                fields.insert("history_reference".into(), reference);
            }
            self.update(vec![entry]);
        }
    }

    pub fn update(&mut self, entries: Vec<Value>) {
        for entry in entries {
            self.entries
                .retain(|old| old.get("path") != entry.get("path"));
            self.entries.push(entry);
        }
    }

    pub fn active(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.get("state").and_then(Value::as_str) != Some("released"))
    }

    pub fn requires_check(&self) -> bool {
        self.entries.iter().any(|entry| {
            entry.get("require_check") == Some(&Value::Bool(true))
                && entry.get("state").and_then(Value::as_str) != Some("released")
        })
    }

    pub fn problem(&self, fresh_check: bool) -> Option<String> {
        let failures = self
            .entries
            .iter()
            .filter(|entry| {
                !matches!(
                    entry.get("state").and_then(Value::as_str),
                    Some("preserved" | "released")
                )
            })
            .collect::<Vec<_>>();
        if !failures.is_empty() {
            let paths = failures
                .iter()
                .take(8)
                .filter_map(|entry| entry.get("path").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!(
                "Completion incomplete: {} registered file protection(s) remain violated or unknown: {paths}. Use protect status and exact guarded restoration; inspection cannot approve these changes.",
                failures.len()
            ))
        } else if self
            .entries
            .iter()
            .any(|entry| entry.get("scope_review_required") == Some(&Value::Bool(true)))
        {
            Some("Completion incomplete: a project operation was not executed because the current request's preservation scope has not been reviewed. Use protect status, compare related unregistered files with the current request, and register any additional required paths before retrying.".into())
        } else if self.requires_check() && !fresh_check {
            Some("Completion incomplete: the registered preservation contract requires a successful check after the last tracked mutation, with complete file comparisons. Use a separate bash check:true command without modifying project files; watch the relevant paths. A command that repairs a project file and then verifies it can pass, but does not satisfy this completion condition.".into())
        } else {
            None
        }
    }

    pub fn preview(&self) -> Value {
        let mut entries = self
            .entries
            .iter()
            .filter(|entry| {
                entry.get("state").and_then(Value::as_str) != Some("released")
                    || entry.get("scope_review_required") == Some(&Value::Bool(true))
            })
            .collect::<Vec<_>>();
        entries
            .sort_by_key(|entry| entry.get("state").and_then(Value::as_str) == Some("preserved"));
        Value::object([
            ("active_count", Value::number(entries.len())),
            (
                "entries",
                Value::Array(
                    entries
                        .into_iter()
                        .take(8)
                        .map(|entry| {
                            Value::Object(
                                [
                                    "path",
                                    "state",
                                    "registered_request",
                                    "history_reference",
                                    "require_check",
                                    "scope_review_required",
                                    "expected_current",
                                    "error",
                                    "registration_incomplete",
                                    "declared_paths",
                                ]
                                .into_iter()
                                .filter_map(|key| {
                                    let value = entry.get(key)?;
                                    if key == "scope_review_required" && value != &Value::Bool(true)
                                    {
                                        return None;
                                    }
                                    Some((key.into(), value.clone()))
                                })
                                .collect(),
                            )
                        })
                        .collect(),
                ),
            ),
            (
                "instruction",
                Value::string(
                    "Recorded protections are native completion conditions. Read/write/review cannot waive them. This is a compact view; protect status or the original history reference returns full baselines and reasons. Only a newer user request can authorize release. Exact restoration uses expected_current and an owned binary baseline; do not retype old contents.",
                ),
            ),
        ])
    }

    pub fn validate(&self, messages: &[Value]) -> Result<(), String> {
        let mut paths = std::collections::BTreeSet::new();
        for entry in &self.entries {
            let path = entry
                .get("path")
                .and_then(Value::as_str)
                .ok_or("Invalid protection path")?;
            if path.is_empty()
                || !paths.insert(path)
                || std::path::Path::new(path)
                    .components()
                    .any(|part| !matches!(part, std::path::Component::Normal(_)))
            {
                return Err("Invalid or duplicate protection path".into());
            }
            if !matches!(
                entry.get("state").and_then(Value::as_str),
                Some("preserved" | "violated" | "unknown" | "released")
            ) {
                return Err("Invalid protection state".into());
            }
            let request = entry
                .get("registered_request")
                .and_then(Value::as_usize)
                .ok_or("Invalid protection request")?;
            if messages
                .get(request)
                .and_then(|message| message.get("role"))
                .and_then(Value::as_str)
                != Some("user")
            {
                return Err("Protection request does not refer to a user message".into());
            }
            let at = entry
                .get("history_reference")
                .and_then(Value::as_str)
                .and_then(|reference| reference.strip_prefix("history:"))
                .and_then(|index| index.parse::<usize>().ok())
                .ok_or("Invalid protection history reference")?;
            if messages
                .get(at)
                .and_then(|message| message.get("role"))
                .and_then(Value::as_str)
                != Some("tool")
            {
                return Err("Protection baseline does not refer to a tool result".into());
            }
            if entry.get("registration_incomplete") == Some(&Value::Bool(true)) {
                continue;
            }
            if !entry
                .get("snapshot_id")
                .and_then(Value::as_str)
                .is_some_and(crate::sessions::valid_id)
                || entry
                    .get("baseline")
                    .and_then(|version| version.get("fingerprint"))
                    .and_then(Value::as_str)
                    .is_none()
            {
                return Err("Invalid protection baseline".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
