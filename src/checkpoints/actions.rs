//! The same small recovery interface for model tools and local commands.
use super::Store;
use super::restore::{Action, RestoreReport};
use super::state::State;
use crate::cancel::Cancellation;
use crate::json::Value;
use std::sync::Arc;

const SHOWN: usize = 50;

pub fn run(store: &Arc<Store>, args: &Value, cancel: &Cancellation) -> Result<Value, String> {
    match string(args, "action")?.ok_or("action is required")? {
        "list" => {
            let limit = match args.get("limit") {
                None => 20,
                Some(value) => value
                    .as_usize()
                    .filter(|n| (1..=200).contains(n))
                    .ok_or("limit must be between 1 and 200")?,
            };
            let items = store
                .list(limit, cancel)?
                .into_iter()
                .map(|item| {
                    Value::object([
                        ("ref", Value::string(item.reference)),
                        ("tool", Value::string(item.tool)),
                        ("created_at", Value::number(item.created)),
                        (
                            "expires_at",
                            Value::number(item.created.saturating_add(super::TTL_MILLIS)),
                        ),
                        ("saved_paths", Value::number(item.saved)),
                        ("partial_count", Value::number(item.partial)),
                    ])
                })
                .collect();
            Ok(Value::object([("calls", Value::Array(items))]))
        }
        "inspect" => {
            let reference = reference(args)?;
            let paths = paths(args)?;
            let result = store.inspect(reference, paths.as_deref(), cancel)?;
            let selected = result
                .selected
                .iter()
                .take(SHOWN)
                .map(|entry| {
                    Value::object([
                        ("path", Value::string(&entry.path)),
                        ("saved", describe(&entry.saved)),
                        ("current", describe(&entry.current)),
                        (
                            "action",
                            entry
                                .action
                                .map_or(Value::Null, |action| Value::string(action.label())),
                        ),
                    ])
                })
                .collect();
            Ok(Value::object([
                ("ref", Value::string(result.reference)),
                ("expected", Value::string(result.expected)),
                ("paths", Value::Array(selected)),
                ("path_count", Value::number(result.selected.len())),
                ("partial", issues(&result.partial)),
                ("partial_count", Value::number(result.partial.len())),
            ]))
        }
        "restore" => {
            let reference = reference(args)?;
            let paths = paths(args)?;
            let preview = boolean(args, "preview")?;
            let expected = string(args, "expected")?
                .ok_or("restore needs expected from inspect for the same paths, or latest to acknowledge the current state")?;
            let report = store.restore(reference, paths.as_deref(), expected, preview, cancel)?;
            Ok(restored(report))
        }
        other => Err(format!(
            "Unknown checkpoint action {other:?}; use list, inspect or restore"
        )),
    }
}

fn restored(report: RestoreReport) -> Value {
    let planned = report
        .planned
        .iter()
        .take(SHOWN)
        .map(|(path, action)| change(path, *action))
        .collect();
    let changed = report
        .restored
        .iter()
        .take(SHOWN)
        .map(|(path, action)| change(path, *action))
        .collect();
    let undo = report.undo.map_or(Value::Null, |undo| {
        Value::object([
            ("action", Value::string("restore")),
            ("ref", Value::string(undo.reference)),
            (
                "paths",
                Value::Array(undo.paths.into_iter().map(Value::string).collect()),
            ),
            ("expected", Value::string(undo.expected)),
        ])
    });
    Value::object([
        ("ref", Value::string(report.reference)),
        ("preview", Value::Bool(report.preview)),
        ("planned", Value::Array(planned)),
        ("planned_count", Value::number(report.planned.len())),
        ("expected", Value::string(report.expected)),
        ("restored", Value::Array(changed)),
        ("restored_count", Value::number(report.restored.len())),
        ("conflicts", issues(&report.conflicts)),
        ("conflict_count", Value::number(report.conflicts.len())),
        ("failed", issues(&report.failed)),
        ("failed_count", Value::number(report.failed.len())),
        ("skipped", issues(&report.skipped)),
        ("skipped_count", Value::number(report.skipped.len())),
        ("undo", undo),
    ])
}

fn change(path: &str, action: Action) -> Value {
    Value::object([
        ("path", Value::string(path)),
        ("action", Value::string(action.label())),
    ])
}

pub(super) fn issues(items: &[super::Issue]) -> Value {
    Value::Array(
        items
            .iter()
            .take(SHOWN)
            .map(|(path, reason)| {
                Value::object([
                    ("path", Value::string(path)),
                    ("reason", Value::string(reason)),
                ])
            })
            .collect(),
    )
}

pub(super) fn describe(state: &State) -> Value {
    let mut fields = vec![("kind", Value::string(state.label()))];
    match state {
        State::File { length, .. } => fields.push(("bytes", Value::number(length))),
        State::Link { target, .. } => fields.push(("target", Value::string(target))),
        State::Unreadable { reason } => fields.push(("reason", Value::string(reason))),
        _ => {}
    }
    Value::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
    )
}

fn string<'a>(args: &'a Value, name: &str) -> Result<Option<&'a str>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| format!("{name} must be a string")),
    }
}

fn reference(args: &Value) -> Result<&str, String> {
    string(args, "ref")?.ok_or_else(|| "ref is required; use a receipt or checkpoint list".into())
}

fn paths(args: &Value) -> Result<Option<Vec<String>>, String> {
    match args.get("paths") {
        None => Ok(None),
        Some(Value::Array(items)) if !items.is_empty() => items.iter()
            .map(|item| item.as_str().map(String::from).ok_or_else(|| "paths must contain strings".into()))
            .collect::<Result<Vec<_>, _>>().map(Some),
        _ => Err("paths must be a nonempty array of file or directory paths; omit it for the call's full scope".into()),
    }
}

fn boolean(args: &Value, name: &str) -> Result<bool, String> {
    match args.get(name) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        _ => Err(format!("{name} must be a boolean")),
    }
}
