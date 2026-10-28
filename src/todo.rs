//! Conversation plans. Successful set results are immutable snapshots in tool history.

use crate::json::{self, Value};
use std::collections::{BTreeMap, BTreeSet};

const MAX_TASKS: usize = 40;
const MAX_SNAPSHOT_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub status: String,
    pub note: String,
    pub agent: Option<String>,
}

impl Task {
    fn parse(value: &Value) -> Result<Self, String> {
        let id = required_text(value, "id", 48)?;
        if !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err("Task id must use letters, digits, '-' or '_'".into());
        }
        let status = required_text(value, "status", 24)?;
        if !matches!(
            status.as_str(),
            "pending" | "in_progress" | "waiting" | "done" | "cancelled"
        ) {
            return Err(format!("Invalid task status: {status}"));
        }
        Ok(Self {
            id,
            title: required_text(value, "title", 200)?,
            status,
            note: optional_text(value, "note", 800)?.unwrap_or_default(),
            agent: optional_text(value, "agent", 80)?,
        })
    }

    pub fn value(&self) -> Value {
        let mut value = Value::object([
            ("id", Value::string(&self.id)),
            ("title", Value::string(&self.title)),
            ("status", Value::string(&self.status)),
            ("note", Value::string(&self.note)),
        ]);
        if let (Some(agent), Value::Object(fields)) = (&self.agent, &mut value) {
            fields.insert("agent".into(), Value::string(agent));
        }
        value
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub title: String,
    pub revision: usize,
    pub tasks: Vec<Task>,
}

impl Plan {
    pub fn empty() -> Self {
        Self {
            title: String::new(),
            revision: 0,
            tasks: Vec::new(),
        }
    }

    pub fn parse(value: &Value) -> Result<Self, String> {
        let revision = value
            .get("revision")
            .and_then(Value::as_usize)
            .ok_or("revision must be a non-negative integer")?;
        let tasks = parse_tasks(value)?;
        let title = if revision == 0
            && tasks.is_empty()
            && value.get("title").and_then(Value::as_str) == Some("")
        {
            String::new()
        } else {
            required_text(value, "title", 200)?
        };
        Ok(Self {
            title,
            revision,
            tasks,
        })
    }

    pub fn set(arguments: &Value, previous: Option<&Self>) -> Result<Self, String> {
        let revision = previous
            .map_or(0, |plan| plan.revision)
            .checked_add(1)
            .ok_or("Plan revision limit reached")?;
        let plan = Self {
            title: required_text(arguments, "title", 200)?,
            revision,
            tasks: parse_tasks(arguments)?,
        };
        if plan.value().encode().len() > MAX_SNAPSHOT_BYTES {
            return Err("Plan snapshot exceeds 32 KiB".into());
        }
        Ok(plan)
    }

    pub fn value(&self) -> Value {
        Value::object([
            ("title", Value::string(&self.title)),
            ("revision", Value::number(self.revision)),
            (
                "tasks",
                Value::Array(self.tasks.iter().map(Task::value).collect()),
            ),
        ])
    }
}

fn required_text(value: &Value, key: &str, limit: usize) -> Result<String, String> {
    let text = value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} must be a string"))?;
    if text.trim().is_empty() || text.len() > limit || text.chars().any(char::is_control) {
        return Err(format!(
            "{key} must be nonempty, single-line text of at most {limit} bytes"
        ));
    }
    Ok(text.to_owned())
}

fn optional_text(value: &Value, key: &str, limit: usize) -> Result<Option<String>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.len() <= limit && !text.chars().any(char::is_control) => {
            Ok(Some(text.clone()))
        }
        _ => Err(format!(
            "{key} must be single-line text of at most {limit} bytes"
        )),
    }
}

fn parse_tasks(value: &Value) -> Result<Vec<Task>, String> {
    let items = value
        .get("tasks")
        .and_then(Value::as_array)
        .ok_or("tasks must be an array")?;
    if items.len() > MAX_TASKS {
        return Err(format!("tasks supports at most {MAX_TASKS} items"));
    }
    let mut ids = BTreeSet::new();
    let mut tasks = Vec::with_capacity(items.len());
    for item in items {
        let task = Task::parse(item)?;
        if !ids.insert(task.id.clone()) {
            return Err(format!("Duplicate task id: {}", task.id));
        }
        tasks.push(task);
    }
    Ok(tasks)
}

/// Find the newest successful set result, including results removed from model context.
pub fn latest(messages: &[Value]) -> Option<Plan> {
    let mut results: BTreeMap<String, &Value> = BTreeMap::new();
    for message in messages.iter().rev() {
        match message.get("role").and_then(Value::as_str) {
            Some("tool") => {
                if let Some(id) = message.get("tool_call_id").and_then(Value::as_str) {
                    results.insert(id.to_owned(), message);
                }
            }
            Some("assistant") => {
                if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
                    for call in calls.iter().rev() {
                        let Ok(call) = crate::conversation::ToolCall::parse(call) else {
                            continue;
                        };
                        if call.name != "todo_list"
                            || json::parse(&call.arguments).ok().is_none_or(|args| {
                                args.get("action").and_then(Value::as_str) != Some("set")
                            })
                        {
                            continue;
                        }
                        let Some(result) = results
                            .get(&call.id)
                            .and_then(|message| message.get("content").and_then(Value::as_str))
                        else {
                            continue;
                        };
                        let Ok(result) = json::parse(result) else {
                            continue;
                        };
                        if result.get("error").is_some() {
                            continue;
                        }
                        let Ok(plan) = Plan::parse(&result) else {
                            continue;
                        };
                        if plan.revision > 0 {
                            return Some(plan);
                        }
                    }
                }
                results.clear();
            }
            _ => results.clear(),
        }
    }
    None
}

#[cfg(test)]
mod tests;
