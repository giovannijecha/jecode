//! The worker holds the exact prepared change; the UI returns only its ID and choice.
use super::{Event, Metrics, worker::Context};
use crate::{
    json::{self, Value},
    tools::{Output, Prepared},
    workspace::{Budget, Workspace},
};
use std::{
    sync::{atomic::Ordering, mpsc::RecvTimeoutError},
    time::{Duration, Instant},
};

pub(super) struct Decision {
    pub id: u64,
    pub allow: bool,
}
pub(super) fn execute(
    tool: Prepared,
    workspace: &Workspace,
    context: &Context,
    deadline: Instant,
    denied: &mut bool,
    metrics: &mut Metrics,
) -> Output {
    let id = context.next_approval.fetch_add(1, Ordering::Relaxed);
    if *denied {
        let output = Output::error(
            "further changes are disabled for this turn after a denial; wait for a new user request",
        );
        let _ = context.send(
            Event::EditFinished {
                id,
                summary: output.summary.clone(),
                applied: false,
                failed: true,
            },
            false,
        );
        return output;
    }
    let budget = Budget {
        cancelled: &context.cancelled,
        deadline: deadline.min(Instant::now() + Duration::from_secs(10)),
    };
    let change = match tool.propose(workspace, &budget) {
        Ok(change) => change,
        Err(error) => {
            let summary = format!("{} / {} / {error}", tool.name(), tool.path());
            let _ = context.send(
                Event::EditFinished {
                    id,
                    summary,
                    applied: false,
                    failed: true,
                },
                false,
            );
            return Output::error(&error.to_string());
        }
    };
    let preview = change.preview().clone();
    // A cancelled older decision can never authorize a later proposal.
    while context.decisions.try_recv().is_ok() {}
    let sent = context
        .send(
            Event::EditProposed {
                id,
                preview: preview.clone(),
            },
            true,
        )
        .is_continue();
    let decision = sent
        .then(|| wait(context, id, deadline, metrics))
        .transpose();
    let (output, applied) = match decision {
        Ok(Some(true)) => {
            let budget = Budget {
                cancelled: &context.cancelled,
                deadline: deadline.min(Instant::now() + Duration::from_secs(10)),
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
                                ("approved", Value::Bool(true)),
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
                Err(error) => (Output::error(&error.to_string()), false),
            }
        }
        Ok(Some(false)) => {
            *denied = true;
            (
                Output::success(
                    json::object([
                        ("ok", Value::Bool(false)),
                        ("status", Value::String("denied".into())),
                        ("approved", Value::Bool(false)),
                        ("path", Value::String(preview.path)),
                    ]),
                    "Denied · no file changed".into(),
                    false,
                ),
                false,
            )
        }
        _ => (
            Output::error("approval cancelled or expired; no file changed"),
            false,
        ),
    };
    let _ = context.send(
        Event::EditFinished {
            id,
            summary: output.summary.clone(),
            applied,
            failed: output.failed,
        },
        false,
    );
    output
}
pub(super) fn wait(
    context: &Context,
    id: u64,
    deadline: Instant,
    metrics: &mut Metrics,
) -> Result<bool, ()> {
    let started = Instant::now();
    let result = decision(context, id, deadline);
    metrics.approval_wait_ms += super::worker::millis(started);
    result
}
fn decision(context: &Context, id: u64, deadline: Instant) -> Result<bool, ()> {
    loop {
        if context.check().is_err() || Instant::now() >= deadline {
            return Err(());
        }
        match context.decisions.recv_timeout(Duration::from_millis(20)) {
            Ok(decision) if decision.id == id => {
                context.check().map_err(|_| ())?;
                return Ok(decision.allow);
            }
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err(()),
        }
    }
}
