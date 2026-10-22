//! The restore actions and their reports, including conflicts, undo refs and
//! partial progress.
use super::super::index::{parse_reference, reference};
use super::super::restore::{self, Expected, Report, Request};
use super::super::session::object;
use super::super::workspace::Workspace;
use super::{Caller, SHOWN, selected, string, strings, target};
use crate::json::Value;
use std::sync::Arc;

pub(super) fn actions(entries: &[(String, restore::Action)]) -> Value {
    let mut shown: Vec<Value> = entries
        .iter()
        .take(SHOWN)
        .map(|(path, action)| {
            Value::object([
                ("path", Value::string(path)),
                ("action", Value::string(action.label())),
            ])
        })
        .collect();
    if entries.len() > SHOWN {
        shown.push(Value::string(format!("+{} more", entries.len() - SHOWN)));
    }
    Value::Array(shown)
}

pub(super) fn skipped(report: &Report) -> Value {
    Value::Array(
        report
            .skipped
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

pub(super) fn restore(
    workspace: &Arc<Workspace>,
    caller: &Caller<'_>,
    arguments: &Value,
    all: bool,
) -> Result<Value, String> {
    let target = target(arguments)?;
    let paths = selected(arguments)?;
    let exact = arguments.get("exact") == Some(&Value::Bool(true));
    if exact && paths.is_none() {
        return Err("exact needs selected paths; use restore to select entries".into());
    }
    match (&paths, all) {
        (Some(_), true) => {
            return Err("restore_all restores the whole scope; use restore to select paths".into());
        }
        (None, false) => {
            return Err("paths is required; use restore_all to restore the whole scope".into());
        }
        _ => {}
    }
    let expected = match string(arguments, "expected")?.filter(|text| !text.is_empty()) {
        Some("latest") => Expected::Latest,
        Some(text) => Expected::At(parse_reference(text)?),
        None if caller.user => Expected::Latest,
        None => Expected::Own,
    };
    let excluded = workspace.excluded.clone();
    let report = workspace.restore(&Request {
        target,
        paths: paths.as_deref(),
        exact,
        expected,
        preview: arguments.get("preview") == Some(&Value::Bool(true)),
        origin: caller.origin,
        excluded: &excluded,
    })?;
    Ok(report_value(&report, all, paths.as_deref(), exact))
}

fn report_value(report: &Report, all: bool, paths: Option<&[String]>, exact: bool) -> Value {
    let mut fields = vec![
        ("target", Value::string(reference(report.target))),
        ("preview", Value::Bool(report.preview)),
    ];
    if report.preview {
        fields.push(("planned", actions(&report.planned)));
    } else {
        fields.push(("restored", actions(&report.applied)));
    }
    if let Some(before) = report.before {
        fields.push(("before", Value::string(reference(before))));
        let partial =
            report.failed.is_some() || !report.conflicts.is_empty() || !report.remaining.is_empty();
        let changed: Vec<String> = report
            .applied
            .iter()
            .map(|(path, _)| path.clone())
            .collect();
        if !partial || !changed.is_empty() {
            let mut undo = vec![
                (
                    "action",
                    Value::string(if partial || paths.is_some() {
                        "restore"
                    } else {
                        "restore_all"
                    }),
                ),
                ("ref", Value::string(reference(before))),
            ];
            if partial {
                undo.push(("paths", strings(changed.iter())));
                undo.push(("exact", Value::Bool(true)));
            } else if let Some(paths) = paths {
                undo.push(("paths", strings(paths.iter())));
                if exact {
                    undo.push(("exact", Value::Bool(true)));
                }
            }
            fields.push(("undo", object(undo)));
        }
    }
    if let Some(after) = report.after {
        fields.push(("after", Value::string(reference(after))));
    }
    if !report.conflicts.is_empty() {
        let conflicts = report
            .conflicts
            .iter()
            .take(SHOWN)
            .map(|conflict| {
                Value::object([
                    ("path", Value::string(&conflict.path)),
                    ("target", Value::string(reference(conflict.target))),
                    ("expected", Value::string(reference(conflict.expected))),
                    ("current", Value::string(reference(conflict.current))),
                ])
            })
            .collect();
        fields.push(("conflicts", Value::Array(conflicts)));
        let current = report
            .conflicts
            .iter()
            .map(|conflict| conflict.current)
            .max()
            .unwrap_or_default();
        let note = if !report.applied.is_empty() {
            "the restore stopped after applying the paths in restored; the conflicting path and remaining paths were not restored"
        } else if all {
            "nothing was restored: a full restore applies only without conflicts"
        } else {
            "the conflicting paths were not restored"
        };
        fields.push((
            "retry",
            Value::object([
                ("expected", Value::string(reference(current))),
                ("note", Value::string(format!("{note}; inspect the current files, then repeat with this expected ref to replace them"))),
            ]),
        ));
    }
    if !report.skipped.is_empty() {
        fields.push(("skipped", skipped(report)));
    }
    if report.unchanged > 0 {
        fields.push(("unchanged", Value::number(report.unchanged)));
    }
    if let Some((path, error)) = &report.failed {
        fields.push((
            "failed",
            Value::object([
                ("path", Value::string(path)),
                ("error", Value::string(error)),
            ]),
        ));
    }
    if !report.remaining.is_empty() {
        fields.push(("not_restored", strings(report.remaining.iter().take(SHOWN))));
    }
    if report.instructions_changed() {
        fields.push(("instructions_changed", Value::Bool(true)));
    }
    object(fields)
}
