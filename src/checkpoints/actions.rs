//! Checkpoint actions shared by the model tool, `jecode checkpoints` and
//! `/checkpoints`. Every action returns structured data; `text` renders it
//! for people.
use super::identity::plain;
use super::index::{parse_reference, reference};
use super::policy::{parse_size, size};
use super::record::State;
use super::restore::{self, Expected, Request};
use super::session::object;
use super::workspace::{Readiness, Watching, Workspace};
use crate::json::Value;
use restoring::{actions, restore, skipped};
use std::sync::Arc;

mod restoring;

/// Actions the model may use; the others belong to the user.
pub const MODEL_ACTIONS: [&str; 7] = [
    "status",
    "capture",
    "list",
    "inspect",
    "milestone",
    "restore",
    "restore_all",
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
        "capture" => super::targeted::run(
            workspace,
            caller.origin,
            &selected(arguments)?.ok_or("capture needs explicit file paths")?,
        ),
        "list" => list(workspace, arguments),
        "inspect" => inspect(workspace, caller, arguments),
        "milestone" => {
            let label = string(arguments, "label")?.ok_or("label is required for a milestone")?;
            let pinned = arguments.get("pin") == Some(&Value::Bool(true));
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
            let limit = |name| string(arguments, name)?.map(parse_size).transpose();
            let policy =
                workspace.configure(limit("budget")?, limit("max_file")?, caller.origin)?;
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
                    Value::string(
                        summary
                            .limited
                            .as_ref()
                            .map_or("full bounded discovery", |_| "selective"),
                    ),
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
        ("max_file", Value::string(size(max_file))),
    ])
}

fn status(workspace: &Arc<Workspace>, own: &str) -> Value {
    let (readiness, watching, running, background, collection, failure) = {
        let health = workspace.signals.lock();
        (
            health.readiness.clone(),
            health.watching.clone(),
            health.bash.len(),
            health.background.values().sum::<usize>(),
            health.collection.clone(),
            health.failure.clone(),
        )
    };
    let mut store = workspace.store();
    let refreshed = store.refresh();
    workspace.observe_coverage(&store);
    let index = &store.index;
    let state = match &readiness {
        Readiness::Starting if !workspace.options().background => {
            "not reconciled (offline command)".to_string()
        }
        Readiness::Starting => "starting".to_string(),
        Readiness::Baseline(count) => format!("first capture running: {count} paths saved"),
        Readiness::Reconciling(count) => {
            format!("reconciling offline changes: {count} paths examined")
        }
        Readiness::Ready if workspace.options().background => "ready".into(),
        Readiness::Ready => "ready (scanned now)".into(),
        Readiness::Failed(reason) => format!("unavailable: {reason}"),
    };
    let state = workspace.selective().map_or(state.clone(), |reason| {
        format!("{state}; selective protection: {reason}")
    });
    let watching = match watching {
        Watching::Active => "native notifications (observed changes; delivery may lag)".to_string(),
        Watching::Off if !workspace.options().background => "off (offline command)".into(),
        Watching::Off | Watching::Starting => "scans at each point".into(),
        Watching::Lost(reason) => format!("restarting after {reason}"),
        Watching::Unavailable(reason) => format!("scans at each point ({reason})"),
    };
    let coverage = match index.current_coverage() {
        Some((true, _)) => "complete".to_string(),
        Some((false, reason)) => format!("incomplete since {reason}"),
        None => "not recorded".into(),
    };
    let mut fields = vec![
        (
            "workspace",
            Value::string(plain(&workspace.root).to_string_lossy()),
        ),
        (
            "archive",
            Value::string(plain(&workspace.archive).to_string_lossy()),
        ),
        ("state", Value::string(state)),
        ("watching", Value::string(watching)),
        ("coverage", Value::string(coverage)),
        ("latest", Value::string(reference(index.latest()))),
        ("oldest_retained", Value::string(reference(index.horizon))),
        ("points", Value::number(index.points.len())),
        ("pinned", Value::number(index.pins.len())),
        (
            "stored",
            Value::string(format!(
                "{} of {}",
                super::quota::used(&workspace.archive).map_or_else(|_| "unknown".into(), size),
                size(super::quota::LIMIT)
            )),
        ),
        (
            "stored_current_generation",
            Value::string(size(store.journal.stored_bytes())),
        ),
        (
            "hard_limits",
            limits(super::quota::LIMIT, super::policy::DEFAULT_MAX_FILE_BYTES),
        ),
        (
            "generation_capacity",
            Value::string(size(super::quota::WORKING_LIMIT)),
        ),
        (
            "compaction_reserve",
            Value::string(size(super::quota::LIMIT - super::quota::WORKING_LIMIT)),
        ),
        (
            "protected_paths",
            Value::number(
                index
                    .paths
                    .values()
                    .filter(|versions| {
                        versions.last().is_some_and(|version| {
                            version.state.exists() && version.state.recoverable()
                        })
                    })
                    .count(),
            ),
        ),
        (
            "uncaptured_content_paths",
            Value::number(
                index
                    .paths
                    .values()
                    .filter(|versions| {
                        versions
                            .last()
                            .is_some_and(|version| !version.state.recoverable())
                    })
                    .count(),
            ),
        ),
        (
            "discovery_limits",
            Value::object([
                ("paths_examined", Value::number(super::inventory::MAX_PATHS)),
                (
                    "baseline_estimate",
                    Value::string(size(store.policy.budget_bytes / 4)),
                ),
            ]),
        ),
        (
            "limits",
            limits(store.policy.budget_bytes, store.policy.max_file_bytes),
        ),
        ("running_commands", Value::number(running)),
        ("background_processes", Value::number(background)),
    ];
    if !index.pending.is_empty() {
        fields.push(("operations_in_progress", Value::number(index.pending.len())));
    }
    let activity: Vec<Value> = super::activity::list(&workspace.archive)
        .into_iter()
        .map(|entry| {
            let who = if entry.origin == own {
                "this conversation".to_string()
            } else {
                entry.origin
            };
            Value::object([
                ("session", Value::string(who)),
                ("state", Value::string(entry.state)),
                ("pid", Value::number(entry.pid)),
                ("updated", Value::string(age(entry.updated))),
            ])
        })
        .collect();
    fields.push(("activity", Value::Array(activity)));
    match collection {
        Some(Ok(collection)) => fields.push((
            "last_collection",
            Value::string(format!(
                "released history before {} ({} to {})",
                reference(collection.horizon),
                size(collection.before),
                size(collection.after)
            )),
        )),
        Some(Err(error)) => {
            fields.push(("last_collection", Value::string(format!("failed: {error}"))))
        }
        None => {}
    }
    if !store.journal.damage.is_empty() {
        fields.push(("damage", strings(store.journal.damage.iter().take(10))));
    }
    if let Some(error) = failure.or(refreshed.err()) {
        fields.push(("error", Value::string(error)));
    }
    object(fields)
}

/// How long ago a millisecond timestamp was.
pub fn age(time: u64) -> String {
    let seconds = crate::sessions::now().saturating_sub(time) / 1000;
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
                        ];
                        if matches!(version.state, State::File { .. }) {
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
                    let state = index.state_at(path, target);
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
