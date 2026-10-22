//! Human and model-readable archive status.
use super::*;
use crate::checkpoints::{activity, quota, recovery};

pub(super) fn status(workspace: &Arc<Workspace>, own: &str) -> Value {
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
        Readiness::Baseline(count) => format!("first capture running: {count} paths saved"),
        Readiness::Reconciling(count) => {
            format!("reconciling offline changes: {count} paths examined")
        }
        Readiness::Ready if workspace.options().background => "ready".into(),
        Readiness::Ready => "ready (on-demand recovery)".into(),
        Readiness::Failed(reason) => format!("unavailable: {reason}"),
    };
    let state = workspace.selective().map_or(state.clone(), |reason| {
        format!("{state}; selective protection: {reason}")
    });
    let watching = match watching {
        Watching::Active => "native notifications (observed changes; delivery may lag)".to_string(),
        Watching::Off if !workspace.options().background => "off (offline command)".into(),
        Watching::Off | Watching::Starting => "on-demand path acquisition".into(),
        Watching::Lost(reason) => format!("restarting after {reason}"),
        Watching::Unavailable(reason) => format!("on-demand path acquisition ({reason})"),
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
            Value::string(quota::used(&workspace.archive).map_or_else(|_| "unknown".into(), size)),
        ),
        (
            "stored_current_generation",
            Value::string(size(store.journal.stored_bytes())),
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
            "limits",
            limits(store.policy.budget_bytes, store.policy.max_file_bytes),
        ),
        (
            "inactive_history",
            Value::string(
                "accepted attempts: at most 24 hours from acceptance; abandoned attempts: configured inactivity period; user pins remain until released",
            ),
        ),
        ("running_commands", Value::number(running)),
        ("background_processes", Value::number(background)),
    ];
    if let Some(group) = recovery::group_for(index, own) {
        fields.push(("active_group", Value::string(reference(group))));
    }
    fields.push((
        "accepted_retention_hours",
        Value::number(store.policy.recovery_accepted_hours),
    ));
    fields.push((
        "inactive_retention_days",
        store
            .policy
            .recovery_inactive_days
            .map_or(Value::Null, Value::number),
    ));
    if !index.pending.is_empty() {
        fields.push(("operations_in_progress", Value::number(index.pending.len())));
    }
    let activity: Vec<Value> = activity::list(&workspace.archive)
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
