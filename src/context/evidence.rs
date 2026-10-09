use super::{protection::Facts, view};
use crate::json::{self, Value};

mod comparisons;

#[cfg(test)]
mod comparison_tests;

#[derive(Clone, Debug, Default)]
pub struct Evidence {
    checks: Vec<Value>,
    last_tool: Option<Value>,
    last_change: Option<Value>,
    uninspected: Vec<Value>,
    protections: Facts,
}

impl Evidence {
    pub fn value(&self) -> Value {
        Value::object([
            ("checks", Value::Array(self.checks.clone())),
            ("last_tool", self.last_tool.clone().unwrap_or(Value::Null)),
            (
                "file_constraints",
                Value::Array(self.protections.entries.clone()),
            ),
            (
                "observed_indirect_changes",
                Value::Array(self.uninspected.clone()),
            ),
            (
                "last_tracked_change",
                self.last_change.clone().unwrap_or(Value::Null),
            ),
        ])
    }

    pub fn prompt_value(&self) -> Value {
        let mut value = self.value();
        if let Value::Object(fields) = &mut value {
            fields.insert("file_constraints".into(), self.protections.preview());
            fields.insert(
                "observed_indirect_change_count".into(),
                Value::number(self.uninspected.len()),
            );
            fields.insert(
                "observed_indirect_changes".into(),
                Value::Array(
                    self.uninspected
                        .iter()
                        .rev()
                        .take(8)
                        .cloned()
                        .map(|mut entry| {
                            if let Value::Object(fields) = &mut entry
                                && let Some(path) = fields.get("path").and_then(Value::as_str)
                            {
                                fields
                                    .insert("path".into(), Value::string(view::excerpt(path, 160)));
                            }
                            entry
                        })
                        .collect(),
                ),
            );
        }
        if let Value::Object(fields) = &mut value {
            fields.insert(
                "uninspected_file_change_count".into(),
                Value::number(
                    self.uninspected
                        .iter()
                        .filter(|entry| entry.get("inspected") != Some(&Value::Bool(true)))
                        .count(),
                ),
            );
        }
        value
    }

    pub fn needs_attention(&self) -> bool {
        self.protection_problem().is_some()
            || self.uninspected.iter().any(|entry| {
                entry.get("inspected") != Some(&Value::Bool(true))
                    || entry.get("completion_reviewed") != Some(&Value::Bool(true))
            })
            || self.checks.last().is_some_and(|check| {
                check.get("after_last_tracked_change") == Some(&Value::Bool(false))
                    || check.get("check_status").and_then(Value::as_str) != Some("passed")
            })
    }

    pub fn completion_notice(&self) -> Option<String> {
        if let Some(problem) = self.protection_problem() {
            return Some(problem);
        }
        let uninspected = self
            .uninspected
            .iter()
            .filter(|entry| entry.get("inspected") != Some(&Value::Bool(true)))
            .count();
        if uninspected > 0 {
            Some(format!(
                "{} observed project file(s) lack a subsequent read/write/edit tool receipt after a change or incomplete comparison. Bash inspections may provide further evidence. See recorded file_changes, file_tracking and history references; command success alone does not prove protected files were preserved.",
                uninspected
            ))
        } else if let Some(check) = self.checks.last()
            && check.get("check_status").and_then(Value::as_str) != Some("passed")
        {
            Some(format!(
                "The latest recorded check did not pass: {} ({}). Report its result accurately; command success elsewhere does not replace it.",
                check
                    .get("check_status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
                check
                    .get("history_reference")
                    .and_then(Value::as_str)
                    .unwrap_or("history unavailable")
            ))
        } else if self.checks.last().is_some_and(|check| {
            check.get("after_last_tracked_change") == Some(&Value::Bool(false))
        }) {
            Some("The latest recorded check predates or itself changed observed project files, or file comparison was incomplete. Further verification may be needed.".into())
        } else {
            None
        }
    }

    pub fn has_file_constraints(&self) -> bool {
        self.protections.active()
    }

    pub fn update_file_constraints(&mut self, entries: Vec<Value>) {
        self.protections.entries = entries;
    }

    pub fn protection_problem(&self) -> Option<String> {
        self.protections
            .problem(self.checks.last().is_some_and(|check| {
                check.get("check_status").and_then(Value::as_str) == Some("passed")
                    && check.get("after_last_tracked_change") == Some(&Value::Bool(true))
            }))
    }

    pub fn finish_review(&mut self) {
        for entry in &mut self.uninspected {
            if entry.get("inspected") == Some(&Value::Bool(true))
                && let Value::Object(fields) = entry
            {
                fields.insert("completion_reviewed".into(), Value::Bool(true));
            }
        }
    }

    pub fn parse(value: Option<&Value>) -> Result<Self, String> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        let checks = value
            .get("checks")
            .and_then(Value::as_array)
            .ok_or("Invalid execution evidence checks")?
            .to_vec();
        if checks.len() > 4 {
            return Err("Invalid execution evidence check count".into());
        }
        let optional = |key| -> Result<Option<Value>, String> {
            match value.get(key) {
                Some(Value::Null) => Ok(None),
                Some(Value::Object(_)) => Ok(value.get(key).cloned()),
                _ => Err(format!("Invalid execution evidence {key}")),
            }
        };
        Ok(Self {
            protections: Facts {
                entries: match value.get("file_constraints") {
                    None => Vec::new(),
                    Some(Value::Array(entries)) => entries.clone(),
                    _ => return Err("Invalid saved file constraints".into()),
                },
            },
            checks,
            last_tool: optional("last_tool")?,
            last_change: optional("last_tracked_change")?,
            uninspected: match value
                .get("observed_indirect_changes")
                .or_else(|| value.get("uninspected_file_changes"))
            {
                None => Vec::new(),
                Some(Value::Array(entries)) => entries.clone(),
                _ => return Err("Invalid uninspected file change evidence".into()),
            },
        })
    }

    pub fn validate(&self, messages: &[Value]) -> Result<(), String> {
        self.protections.validate(messages)?;
        for receipt in self
            .checks
            .iter()
            .chain(&self.last_tool)
            .chain(&self.last_change)
            .chain(&self.uninspected)
        {
            let at = receipt
                .get("message")
                .and_then(Value::as_usize)
                .ok_or("Invalid execution evidence message")?;
            if messages
                .get(at)
                .and_then(|message| message.get("role"))
                .and_then(Value::as_str)
                != Some("tool")
            {
                return Err("Execution evidence does not reference a tool result".into());
            }
        }
        Ok(())
    }

    pub fn record(&mut self, at: usize, name: &str, arguments: &Value, result: &Value) {
        self.protections.record(at, result);
        let target = arguments
            .get(if name == "bash" { "command" } else { "path" })
            .and_then(Value::as_str)
            .unwrap_or("");
        let status = if result.get("cancelled") == Some(&Value::Bool(true)) {
            "cancelled"
        } else if result.get("timed_out") == Some(&Value::Bool(true)) {
            "timed_out"
        } else if result.get("outcome").and_then(Value::as_str) == Some("unknown") {
            "unknown"
        } else if result.get("error").is_some() {
            "failed_or_not_executed"
        } else if name == "bash" {
            if result.get("exit_code").and_then(Value::as_usize) == Some(0) {
                "exit_0"
            } else if result
                .get("exit_code")
                .is_some_and(|exit| *exit != Value::Null)
            {
                "nonzero_exit"
            } else {
                "unknown"
            }
        } else {
            "returned"
        };
        let check = name == "bash" && result.get("check") == Some(&Value::Bool(true));
        let mut receipt = Value::object([
            ("message", Value::number(at)),
            ("history_reference", Value::string(format!("history:{at}"))),
            ("tool", Value::string(name)),
            ("target", Value::string(view::excerpt(target, 160))),
            ("status", Value::string(status)),
        ]);
        let changes = result
            .get("file_changes")
            .and_then(Value::as_array)
            .unwrap_or(&[]);
        let incomplete = result
            .get("file_tracking")
            .and_then(|tracking| tracking.get("status"))
            .and_then(Value::as_str)
            == Some("incomplete");
        let explicit_change = matches!(name, "write" | "edit" | "protect")
            && result.get("bytes_written").is_some()
            && status == "returned"
            && result.get("file_scope").and_then(Value::as_str) != Some("temporary")
            && !target.starts_with("tmp:");
        if !changes.is_empty() || incomplete || explicit_change {
            if let Value::Object(fields) = &mut receipt {
                fields.insert("changed_file_count".into(), Value::number(changes.len()));
                fields.insert(
                    "changed_files".into(),
                    Value::Array(
                        changes
                            .iter()
                            .take(8)
                            .filter_map(|change| {
                                change
                                    .get("path")
                                    .and_then(Value::as_str)
                                    .map(|path| Value::string(view::excerpt(path, 160)))
                            })
                            .collect(),
                    ),
                );
                fields.insert("file_comparison_complete".into(), Value::Bool(!incomplete));
            }
            self.last_change = Some(receipt.clone());
            for old in &mut self.checks {
                if let Value::Object(fields) = old {
                    fields.insert("after_last_tracked_change".into(), Value::Bool(false));
                }
            }
        }
        for change in changes {
            let Some(path) = change.get("path").and_then(Value::as_str) else {
                continue;
            };
            self.uninspected
                .retain(|entry| entry.get("path").and_then(Value::as_str) != Some(path));
            if change.get("source").and_then(Value::as_str) != Some("file_tool")
                && change.get("restored_to_first_observed") != Some(&Value::Bool(true))
            {
                self.uninspected.push(Value::object([
                    ("path", Value::string(path)),
                    ("message", Value::number(at)),
                    ("history_reference", Value::string(format!("history:{at}"))),
                    ("state", Value::string("changed")),
                    ("inspected", Value::Bool(false)),
                ]));
            }
        }
        if let Some(errors) = result
            .get("file_tracking")
            .and_then(|tracking| tracking.get("errors"))
            .and_then(Value::as_array)
        {
            for error in errors {
                let Some(path) = error.get("path").and_then(Value::as_str) else {
                    continue;
                };
                self.uninspected
                    .retain(|entry| entry.get("path").and_then(Value::as_str) != Some(path));
                self.uninspected.push(Value::object([
                    ("path", Value::string(path)),
                    ("message", Value::number(at)),
                    ("history_reference", Value::string(format!("history:{at}"))),
                    ("state", Value::string("comparison_incomplete")),
                    ("inspected", Value::Bool(false)),
                ]));
            }
        }
        if matches!(name, "read" | "write" | "edit") {
            for observation in result
                .get("file_observations")
                .and_then(Value::as_array)
                .unwrap_or(&[])
            {
                if let Some(path) = observation.get("path").and_then(Value::as_str) {
                    if name == "read" {
                        for entry in &mut self.uninspected {
                            if entry.get("path").and_then(Value::as_str) == Some(path)
                                && let Value::Object(fields) = entry
                            {
                                fields.insert("inspected".into(), Value::Bool(true));
                                fields.insert(
                                    "inspection_history_reference".into(),
                                    Value::string(format!("history:{at}")),
                                );
                            }
                        }
                    } else {
                        self.uninspected.retain(|entry| {
                            entry.get("path").and_then(Value::as_str) != Some(path)
                        });
                    }
                }
            }
        }
        self.reconcile_comparisons(at, name, arguments, result);
        if check && let Value::Object(fields) = &mut receipt {
            fields.insert(
                "check_status".into(),
                result
                    .get("check_status")
                    .cloned()
                    .unwrap_or(Value::string("unknown")),
            );
            fields.insert(
                "after_last_tracked_change".into(),
                Value::Bool(
                    !incomplete
                        && changes.iter().all(|change| {
                            change.get("source").and_then(Value::as_str) == Some("before_command")
                        })
                        && (self
                            .last_change
                            .as_ref()
                            .and_then(|entry| entry.get("message"))
                            .and_then(Value::as_usize)
                            .is_none_or(|change| at >= change)),
                ),
            );
            self.checks.push(receipt.clone());
            if self.checks.len() > 4 {
                self.checks.remove(0);
            }
        }
        self.last_tool = Some(receipt);
    }

    pub fn rebuild(&mut self, messages: &[Value]) {
        let reviewed = self
            .uninspected
            .iter()
            .filter(|entry| entry.get("completion_reviewed") == Some(&Value::Bool(true)))
            .cloned()
            .collect::<Vec<_>>();
        *self = Self::default();
        let mut calls = std::collections::BTreeMap::new();
        for (index, message) in messages.iter().enumerate() {
            if let Some(batch) = message.get("tool_calls").and_then(Value::as_array) {
                for call in batch {
                    if let Some(id) = call.get("id").and_then(Value::as_str) {
                        calls.insert(id, call);
                    }
                }
            }
            if message.get("role").and_then(Value::as_str) == Some("tool")
                && let Some(call) = message
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .and_then(|id| calls.get(id))
                && let Some(function) = call.get("function")
                && let Some(name) = function.get("name").and_then(Value::as_str)
                && let Some(arguments) = function
                    .get("arguments")
                    .and_then(Value::as_str)
                    .and_then(|text| json::parse(text).ok())
                && let Some(result) = message
                    .get("content")
                    .and_then(Value::as_str)
                    .and_then(|text| json::parse(text).ok())
            {
                self.record(index, name, &arguments, &result);
            }
        }
        for entry in &mut self.uninspected {
            if reviewed.iter().any(|old| {
                old.get("path") == entry.get("path") && old.get("message") == entry.get("message")
            }) && let Value::Object(fields) = entry
            {
                fields.insert("completion_reviewed".into(), Value::Bool(true));
            }
        }
    }
}

#[cfg(test)]
mod tests;
