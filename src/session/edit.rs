//! Prepare and apply one file change, reporting the exact result to the controller.
use super::{Event, worker::Context};
use crate::{
    json::{self, Value},
    tools::{Output, Prepared},
    workspace::{Budget, Workspace},
};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

pub(super) fn execute(tool: Prepared, workspace: &Workspace, context: &Context) -> Output {
    let id = context.next_effect.fetch_add(1, Ordering::Relaxed);
    let budget = Budget {
        cancelled: &context.cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let change = match tool.propose(workspace, &budget) {
        Ok(change) => change,
        Err(error) => {
            let output = Output::not_executed(&error.to_string());
            finished(context, id, &output, false);
            return output;
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
            deadline: Instant::now() + Duration::from_secs(10),
        };
        match workspace.apply(change, &budget) {
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
                (
                    Output::success(
                        json::object([
                            ("ok", Value::Bool(true)),
                            ("status", Value::String("applied".into())),
                            ("path", Value::String(preview.path)),
                            (
                                "recovery",
                                result.recovery.map_or(Value::Null, Value::String),
                            ),
                        ]),
                        summary,
                        false,
                    ),
                    true,
                )
            }
            Err(error) => (Output::failed_effect(&error.to_string()), false),
        }
    };
    finished(context, id, &output, applied);
    output
}

fn finished(context: &Context, id: u64, output: &Output, applied: bool) {
    let _ = context.send(
        Event::EditFinished {
            id,
            summary: output.summary.clone(),
            applied,
            failed: output.failed,
        },
        false,
    );
}
