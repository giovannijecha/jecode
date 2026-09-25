//! Prepare and apply one file change, reporting the exact result to the controller.
use super::{Event, worker::Context};
use crate::{
    json::{self, Value},
    tools::{Output, Prepared},
    workspace::{Budget, RecoveryStore, Workspace},
};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

pub(super) fn execute(
    tool: Prepared,
    workspace: &Workspace,
    context: &Context,
    recoveries: &RecoveryStore,
    session: Option<&str>,
    operation: &str,
) -> (Output, Event) {
    let id = context.next_effect.fetch_add(1, Ordering::Relaxed);
    let budget = Budget {
        cancelled: &context.cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let change = match tool.propose(workspace, &budget) {
        Ok(change) => change,
        Err(error) => {
            let output = Output::not_executed(&error.to_string());
            let finished = finished(id, &output, false);
            return (output, finished);
        }
    };
    let preview = change.preview().clone();
    let planned = context.send(
        Event::EditPlanned {
            id,
            preview: preview.clone(),
        },
        true,
    );
    #[cfg(test)]
    if planned.is_continue() {
        context.before_effect(tool.name());
    }
    let (output, applied) = if planned.is_break() || context.check().is_err() {
        (
            Output::not_executed("file change cancelled before execution; no file changed"),
            false,
        )
    } else {
        let budget = Budget {
            cancelled: &context.cancelled,
            deadline: Instant::now() + Duration::from_secs(30),
        };
        match workspace.apply(change, &budget, recoveries, session, operation) {
            Ok(result) => {
                let summary = format!(
                    "{} {} · +{} -{}{}",
                    if preview.create { "Created" } else { "Edited" },
                    preview.path,
                    preview.added,
                    preview.removed,
                    result
                        .recovery
                        .as_ref()
                        .map_or(String::new(), |path| format!("\n  Recovery: {path}"))
                );
                let mut output = Output::success(
                    json::object([
                        ("ok", Value::Bool(true)),
                        ("status", Value::String("applied".into())),
                        ("path", Value::String(preview.path)),
                        (
                            "recovery",
                            result.recovery.map_or(Value::Null, Value::String),
                        ),
                        (
                            "warning",
                            result
                                .warning
                                .as_deref()
                                .map_or(Value::Null, |message| Value::String(message.into())),
                        ),
                    ]),
                    summary,
                    false,
                );
                if let Some(warning) = result.warning {
                    output.summary.push_str(&format!("\n  {warning}"));
                }
                output.stop_after = result.stop_after;
                (output, true)
            }
            Err(error) => (
                Output::failed_effect_with_recovery(&error.to_string(), error.1.as_deref()),
                false,
            ),
        }
    };
    let finished = finished(id, &output, applied);
    (output, finished)
}

fn finished(id: u64, output: &Output, applied: bool) -> Event {
    Event::EditFinished {
        id,
        summary: output.summary.clone(),
        applied,
        failed: output.failed,
    }
}
