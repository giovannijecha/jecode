//! Model-requested preimages share Bash's precise path and directory capture.
use super::index::reference;
use super::recovery;
use super::scopes::{self, Captured};
use super::workspace::Workspace;
use crate::cancel::Cancellation;
use crate::json::Value;
use std::sync::Arc;

#[cfg(test)]
mod tests;

fn within(root: &str, path: &str) -> bool {
    root.is_empty()
        || path
            .strip_prefix(root)
            .is_some_and(|tail| tail.is_empty() || tail.starts_with('/'))
}

fn normalized(workspace: &Workspace, path: &str) -> String {
    if matches!(path, "." | "./") {
        return String::new();
    }
    let path = path.replace('\\', "/").trim_end_matches('/').to_string();
    super::identity::relative(
        &workspace.root,
        &super::identity::absolute(&workspace.root, &path),
    )
    .unwrap_or(path)
}

fn entry(path: &str, captured: &Captured, workspace: &Workspace) -> Value {
    let name = normalized(workspace, path);
    let directory = captured
        .selection
        .directories
        .iter()
        .any(|root| root == &name);
    let workspace_root = directory && name.is_empty();
    let reason = captured.partial.iter().find(|(affected, _)| {
        affected == path || affected == &name || directory && within(&name, affected)
    });
    let state = captured
        .snapshot
        .filter(|_| !workspace_root)
        .and_then(|before| {
            let store = workspace.store();
            recovery::target(&store.index, before, &name)
        });
    let covered = directory && captured.complete_dirs.contains(&name) || state.is_some();
    let skipped = reason.is_some_and(|(_, reason)| {
        reason == "outside the checkpoint scope" || reason.starts_with("Invalid restore path")
    });
    let status = match (covered, reason) {
        (true, Some(_)) => "partial",
        (true, None) => "saved",
        (false, Some(_)) if skipped => "skipped",
        (false, Some(_)) => "unprotected",
        (false, None) => "skipped",
    };
    let mut fields = vec![
        ("path", Value::string(path)),
        ("status", Value::string(status)),
    ];
    if workspace_root {
        fields.push(("state", Value::string("directory scope")));
    } else if let Some(state) = state {
        fields.push(("state", Value::string(state.label())));
    }
    if let Some((_, reason)) = reason {
        fields.push(("reason", Value::string(reason)));
    }
    super::session::object(fields)
}

/// Every `before` reference names a durable targeted recovery snapshot.
pub fn run(workspace: &Arc<Workspace>, origin: &str, paths: &[String]) -> Result<Value, String> {
    if paths.is_empty() {
        return Err("capture needs explicit paths".into());
    }
    let captured = match scopes::capture(workspace, paths, origin, &Cancellation::default()) {
        Ok(captured) => captured,
        Err(reason) => {
            return Ok(Value::object([
                ("status", Value::string("none")),
                (
                    "paths",
                    Value::Array(
                        paths
                            .iter()
                            .map(|path| {
                                Value::object([
                                    ("path", Value::string(path)),
                                    ("status", Value::string("unprotected")),
                                    ("reason", Value::string(&reason)),
                                ])
                            })
                            .collect(),
                    ),
                ),
                ("saved", Value::number(0)),
                ("skipped", Value::number(0)),
                ("unprotected", Value::number(paths.len())),
                ("reason", Value::string(reason)),
            ]));
        }
    };
    let entries: Vec<Value> = paths
        .iter()
        .map(|path| entry(path, &captured, workspace))
        .collect();
    let skipped = entries
        .iter()
        .filter(|entry| entry.get("status").and_then(Value::as_str) == Some("skipped"))
        .count();
    let unprotected = entries
        .iter()
        .filter(|entry| entry.get("status").and_then(Value::as_str) == Some("unprotected"))
        .count();
    let status = if captured.saved == 0 && captured.complete_dirs.is_empty() {
        "none"
    } else if captured.partial.is_empty() {
        "saved"
    } else {
        "partial"
    };
    let mut fields = vec![
        ("status", Value::string(status)),
        ("paths", Value::Array(entries)),
        ("saved", Value::number(captured.saved)),
        ("skipped", Value::number(skipped)),
        ("unprotected", Value::number(unprotected)),
        (
            "partial",
            Value::Array(
                captured
                    .partial
                    .iter()
                    .map(|(path, reason)| {
                        Value::object([
                            ("path", Value::string(path)),
                            ("reason", Value::string(reason)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ];
    if let Some(group) = captured.group {
        fields.push(("group", Value::string(reference(group))));
    }
    if let Some(before) = captured.snapshot {
        fields.push(("before", Value::string(reference(before))));
        fields.push(("ref", Value::string(reference(before))));
    }
    Ok(super::session::object(fields))
}
