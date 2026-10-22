//! Checkpoint actions shared by the model tool, `jecode checkpoints` and
//! `/checkpoints`. Every action returns structured data; `text` renders it
//! for people.
use super::identity::plain;
use super::index::{parse_reference, reference};
use super::policy::{file_limit, parse_size, size};
use super::record::State;
use super::restore::{self, Expected, Request};
use super::session::object;
use super::workspace::{Readiness, Watching, Workspace};
use crate::json::Value;
use restoring::{actions, restore, skipped};
use std::sync::Arc;

mod recovering;
mod status;
use status::status;
mod restoring;

/// Actions the model may use; the others belong to the user.
pub const MODEL_ACTIONS: [&str; 8] = [
    "status",
    "capture",
    "list",
    "inspect",
    "milestone",
    "restore",
    "restore_all",
    "accept",
];

const LIST_DEFAULT: usize = 20;
const LIST_MAX: usize = 200;
const SHOWN: usize = 50;

pub struct Caller<'a> {
    pub origin: &'a str,
    /// The user decides with full authority: restores replace the latest
    /// state unless an explicit precondition is given.
    pub user: bool,
}

pub fn run(
    workspace: &Arc<Workspace>,
    caller: &Caller<'_>,
    arguments: &Value,
) -> Result<Value, String> {
    super::lifecycle::touch_throttled(&workspace.archive)?;
    let action = arguments
        .get("action")
        .and_then(Value::as_str)
        .ok_or("action must be a string")?;
    if !caller.user && !MODEL_ACTIONS.contains(&action) {
        return Err(format!(
            "Unknown checkpoint action {action:?}; use one of {}",
            MODEL_ACTIONS.join(", ")
        ));
    }
    match action {
        "status" => Ok(status(workspace, caller.origin)),
        "capture" => recovering::capture(workspace, caller, arguments),
        "accept" => recovering::accept(workspace, caller, arguments),
        "list" => list(workspace, arguments),
        "inspect" => inspect(workspace, caller, arguments),
        "milestone" => {
            let label = string(arguments, "label")?.ok_or("label is required for a milestone")?;
            let pinned = arguments.get("pin") == Some(&Value::Bool(true));
            if pinned && !caller.user {
                return Err("Only the user can pin recovery history".into());
            }
            let seq = workspace.mark("milestone", label, caller.origin, pinned, false)?;
            Ok(Value::object([
                ("ref", Value::string(reference(seq))),
                ("label", Value::string(label)),
                ("pinned", Value::Bool(pinned)),
            ]))
        }
        "restore" | "restore_all" => restore(workspace, caller, arguments, action == "restore_all"),
        "pin" | "unpin" => {
            let target = target(arguments)?;
            let label = string(arguments, "label")?.unwrap_or("");
            workspace.pin(target, label, action == "pin")?;
            Ok(Value::object([
                ("ref", Value::string(reference(target))),
                ("pinned", Value::Bool(action == "pin")),
            ]))
        }
        "config" => {
            let budget = string(arguments, "budget")?.map(parse_size).transpose()?;
            let max_file = string(arguments, "max_file")?
                .map(|value| {
                    if value.eq_ignore_ascii_case("unlimited") || value.eq_ignore_ascii_case("none")
                    {
                        Ok(u64::MAX)
                    } else {
                        parse_size(value)
                    }
                })
                .transpose()?;
            let policy = workspace.configure(budget, max_file, caller.origin)?;
            Ok(limits(policy.budget_bytes, policy.max_file_bytes))
        }
        "gc" => match workspace.collect(true)? {
            Some(collection) => Ok(Value::object([
                (
                    "oldest_retained",
                    Value::string(reference(collection.horizon)),
                ),
                ("stored_before", Value::string(size(collection.before))),
                ("stored_after", Value::string(size(collection.after))),
                ("over_budget", Value::Bool(collection.over_budget)),
            ])),
            None => Ok(Value::object([(
                "result",
                Value::string("another process is compacting the archive"),
            )])),
        },
        "verify" => {
            let summary = workspace.verify()?;
            let mut changed: Vec<&String> = summary.changed.iter().map(|(path, _)| path).collect();
            changed.sort();
            Ok(Value::object([
                ("scanned", Value::number(summary.scanned)),
                ("reread", Value::number(summary.candidates)),
                ("excluded", Value::number(summary.excluded)),
                ("unsupported", Value::number(summary.unsupported)),
                ("unreadable_directories", Value::number(summary.unreadable)),
                (
                    "coverage",
                    Value::string(summary.limited.as_ref().map_or("complete", |_| "selective")),
                ),
                (
                    "coverage_reason",
                    summary.limited.as_ref().map_or(Value::Null, Value::string),
                ),
                ("changed", strings(changed.iter().copied().take(SHOWN))),
                ("changed_count", Value::number(changed.len())),
            ]))
        }
        _ => Err(format!("Unknown checkpoint action {action:?}")),
    }
}

fn string<'a>(arguments: &'a Value, name: &str) -> Result<Option<&'a str>, String> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| format!("{name} must be a string")),
    }
}

fn target(arguments: &Value) -> Result<u64, String> {
    parse_reference(string(arguments, "ref")?.ok_or("ref is required, such as cp-12")?)
}

fn strings<'a>(items: impl Iterator<Item = &'a String>) -> Value {
    Value::Array(items.map(Value::string).collect())
}

fn limits(budget: u64, max_file: u64) -> Value {
    Value::object([
        ("budget", Value::string(size(budget))),
        ("budget_kind", Value::string("soft retention target")),
        ("max_file", Value::string(file_limit(max_file))),
    ])
}

/// How long ago a millisecond timestamp was.
pub fn age(time: u64) -> String {
    let seconds = crate::checkpoints::clock::now().saturating_sub(time) / 1000;
    match seconds {
        0..60 => format!("{seconds}s ago"),
        60..3600 => format!("{}m ago", seconds / 60),
        3600..86400 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86400),
    }
}

fn limit(arguments: &Value) -> Result<usize, String> {
    match arguments.get("limit") {
        None => Ok(LIST_DEFAULT),
        Some(value) => value
            .as_usize()
            .filter(|limit| (1..=LIST_MAX).contains(limit))
            .ok_or_else(|| format!("limit must be between 1 and {LIST_MAX}")),
    }
}

fn selected(arguments: &Value) -> Result<Option<Vec<String>>, String> {
    let Some(value) = arguments.get("paths") else {
        return Ok(None);
    };
    let paths = value
        .as_array()
        .ok_or("paths must be an array of workspace-relative paths")?;
    if paths.is_empty() {
        return Ok(None);
    }
    paths
        .iter()
        .map(|path| {
            path.as_str()
                .ok_or("paths must contain strings".to_string())
                .and_then(restore::normalize)
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn list(workspace: &Arc<Workspace>, arguments: &Value) -> Result<Value, String> {
    let limit = limit(arguments)?;
    let paths = selected(arguments)?;
    let mut store = workspace.store();
    store.refresh()?;
    let index = &store.index;
    if let Some(paths) = paths {
        // The recorded versions of each path, newest first.
        let files = paths
            .iter()
            .map(|path| {
                let versions = index.paths.get(path).map(Vec::as_slice).unwrap_or_default();
                let shown: Vec<Value> = versions
                    .iter()
                    .rev()
                    .take(limit)
                    .map(|version| {
                        let mut fields = vec![
                            ("ref", Value::string(reference(version.seq))),
                            ("state", Value::string(describe(&version.state))),
                            ("origin", Value::string(&version.origin)),
                            ("time", Value::string(age(version.time))),
                            (
                                "available",
                                Value::Bool(
                                    index.retained(version.seq) && version.state.recoverable(),
                                ),
                            ),
                        ];
                        if matches!(version.state, State::File { .. })
                            && index.retained(version.seq)
                        {
                            fields.push((
                                "read",
                                Value::string(format!(
                                    "checkpoint:{}:{path}",
                                    reference(version.seq)
                                )),
                            ));
                        }
                        object(fields)
                    })
                    .collect();
                Value::object([
                    ("path", Value::string(path)),
                    ("versions", Value::Array(shown)),
                    ("total", Value::number(versions.len())),
                ])
            })
            .collect();
        return Ok(Value::object([
            ("paths", Value::Array(files)),
            ("oldest_retained", Value::string(reference(index.horizon))),
        ]));
    }
    let mut previous = index.horizon;
    let mut points: Vec<Value> = index
        .points
        .values()
        .filter(|point| index.retained(point.seq))
        .map(|point| {
            let changes = index.changed_between(previous, point.seq).len();
            previous = point.seq;
            let mut fields = vec![
                ("ref", Value::string(reference(point.seq))),
                ("kind", Value::string(&point.kind)),
                ("label", Value::string(&point.label)),
                ("origin", Value::string(&point.origin)),
                ("time", Value::string(age(point.time))),
                ("changes", Value::number(changes)),
            ];
            if index.pins.contains_key(&point.seq) {
                fields.push(("pinned", Value::Bool(true)));
            }
            object(fields)
        })
        .collect();
    let total = points.len();
    points.reverse();
    points.truncate(limit);
    Ok(Value::object([
        ("points", Value::Array(points)),
        ("total", Value::number(total)),
        ("oldest_retained", Value::string(reference(index.horizon))),
    ]))
}

pub fn describe(state: &State) -> String {
    match state {
        State::File { length, .. } => format!("file, {}", size(*length)),
        State::Link { target } => format!("link to {target}"),
        State::Oversized { length } => {
            format!("not captured: {} exceeded the file limit", size(*length))
        }
        State::Unreadable { reason } => format!("not captured: {reason}"),
        other => other.label().into(),
    }
}

fn inspect(
    workspace: &Arc<Workspace>,
    caller: &Caller<'_>,
    arguments: &Value,
) -> Result<Value, String> {
    let target = target(arguments)?;
    let paths = selected(arguments)?;
    let mut fields = Vec::new();
    {
        let mut store = workspace.store();
        store.refresh()?;
        let index = &store.index;
        if let Some(point) = index.points.get(&target) {
            fields.push(("kind", Value::string(&point.kind)));
            fields.push(("label", Value::string(&point.label)));
            fields.push(("origin", Value::string(&point.origin)));
            fields.push(("time", Value::string(age(point.time))));
            if let Some(group) = index.recovery.groups.get(&target) {
                fields.push(("created_at", Value::number(group.created)));
                fields.push(("last_activity", Value::number(group.last_activity)));
                fields.push((
                    "accepted_at",
                    group.accepted.map_or(Value::Null, Value::number),
                ));
                fields.push((
                    "expires_at",
                    group.accepted_until.map_or(Value::Null, Value::number),
                ));
            }
            if let Some(snapshot) = index.recovery.snapshots.get(&target) {
                fields.push(("group", Value::string(reference(snapshot.group))));
                fields.push(("captured_at", Value::number(snapshot.time)));
            }
            let previous = index
                .points
                .range(..target)
                .next_back()
                .map_or(index.horizon, |(seq, _)| *seq);
            let changed: Vec<String> = index
                .changed_between(previous, target)
                .into_iter()
                .map(|version| version.path.clone())
                .collect();
            fields.push((
                "changed_since_previous_point",
                strings(changed.iter().take(SHOWN)),
            ));
        }
        if let Some(paths) = &paths {
            let states: Vec<Value> = paths
                .iter()
                .map(|path| {
                    let state = restore::target_state(index, target, path).unwrap_or_else(|| {
                        State::Unreadable {
                            reason: "not protected by this recovery point".into(),
                        }
                    });
                    let mut entry = vec![
                        ("path", Value::string(path)),
                        ("at_ref", Value::string(describe(&state))),
                        (
                            "current",
                            Value::string(describe(&index.current_state(path))),
                        ),
                    ];
                    if matches!(state, State::File { .. }) {
                        entry.push((
                            "read",
                            Value::string(format!("checkpoint:{}:{path}", reference(target))),
                        ));
                    }
                    object(entry)
                })
                .collect();
            fields.push(("paths", Value::Array(states)));
        }
    }
    // What restoring would change now.
    let excluded = workspace.excluded.clone();
    let preview = workspace.restore(&Request {
        target,
        paths: paths.as_deref(),
        exact: arguments.get("exact") == Some(&Value::Bool(true)),
        expected: Expected::Latest,
        preview: true,
        origin: caller.origin,
        excluded: &excluded,
    })?;
    fields.insert(0, ("ref", Value::string(reference(target))));
    fields.push(("restore_would", actions(&preview.planned)));
    if !preview.skipped.is_empty() {
        fields.push(("restore_would_skip", skipped(&preview)));
    }
    Ok(object(fields))
}
