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

    /// Every task is done or cancelled; the next set starts a new plan.
    pub fn finished(&self) -> bool {
        self.tasks
            .iter()
            .all(|task| matches!(task.status.as_str(), "done" | "cancelled"))
    }
}

/// Whether a todo_list call creates a plan after `current`: a set when there
/// is no plan yet or the previous one finished. Reads, updates and replans
/// of an open plan do not.
pub fn creates(current: Option<&Plan>, arguments: &Value) -> bool {
    arguments.get("action").and_then(Value::as_str) == Some("set")
        && current.is_none_or(Plan::finished)
}

/// A task's phase as the user reads it: the main agent's mark, joined with its
/// collaborator's lifecycle while the main agent has not marked the work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    NotStarted,
    InProgress,
    Waiting,
    AwaitingReview,
    Failed,
    Done,
    Cancelled,
}

impl Phase {
    /// `owner` is the task's collaborator record from the runtime, if any.
    pub fn of(task: &Task, owner: Option<&Value>) -> Self {
        let lifecycle = owner.and_then(|record| record.get("state").and_then(Value::as_str));
        match (task.status.as_str(), lifecycle) {
            ("done", _) => Self::Done,
            ("cancelled", _) => Self::Cancelled,
            ("waiting", _) => Self::Waiting,
            ("pending" | "in_progress", Some("returned")) => Self::AwaitingReview,
            ("pending" | "in_progress", Some("failed")) => Self::Failed,
            ("in_progress", _) => Self::InProgress,
            _ => Self::NotStarted,
        }
    }

    /// The state word; not started has none. `narrow` shortens review.
    pub fn word(self, narrow: bool) -> &'static str {
        match self {
            Self::NotStarted => "",
            Self::InProgress => "in progress",
            Self::Waiting => "waiting",
            Self::AwaitingReview if narrow => "review",
            Self::AwaitingReview => "awaiting review",
            Self::Failed => "failed",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn finished(self) -> bool {
        matches!(self, Self::Done | Self::Cancelled)
    }
}

/// The task's collaborator reference; blank means the main agent.
fn agent(task: &Task) -> Option<&str> {
    task.agent
        .as_deref()
        .map(str::trim)
        .filter(|agent| !agent.is_empty())
}

/// The collaborator record a task names, by ID (`a2`) or number.
pub fn owner<'a>(task: &Task, records: &'a [Value]) -> Option<&'a Value> {
    let agent = agent(task)?;
    let number = agent_number(agent);
    records
        .iter()
        .find(|record| record.get("id").and_then(Value::as_str) == Some(agent))
        .or_else(|| {
            // A closed collaborator's number is reused; the latest holds it.
            records.iter().rev().find(|record| {
                number.is_some() && record.get("number").and_then(Value::as_usize) == number
            })
        })
}

/// `AGENT N` for a task's collaborator, or its own reference in capitals.
pub fn owner_label(task: &Task, owner: Option<&Value>) -> Option<String> {
    let agent = agent(task)?;
    let number = owner
        .and_then(|record| record.get("number").and_then(Value::as_usize))
        .or_else(|| agent_number(agent));
    Some(number.map_or_else(
        || agent.chars().take(16).collect::<String>().to_uppercase(),
        |number| format!("AGENT {number}"),
    ))
}

/// `a3`, `AGENT 3`, `agent-3` and `3` name collaborator number 3.
fn agent_number(agent: &str) -> Option<usize> {
    let lower = agent.to_ascii_lowercase();
    let digits = lower
        .strip_prefix("agent")
        .map(|rest| rest.trim_start_matches([' ', '-', '_']))
        .or_else(|| lower.strip_prefix('a'))
        .unwrap_or(&lower);
    digits
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| digits.parse().ok())
        .flatten()
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
