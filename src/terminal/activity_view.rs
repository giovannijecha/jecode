//! Runtime state belongs to the transient transcript tail above the input rule.
use super::{
    model::Model,
    style::{Row, Tone, lines},
};

pub(super) fn rows(model: &Model, width: usize, available: usize) -> Vec<Row> {
    if model
        .action_demo
        .as_ref()
        .is_some_and(|demo| demo.pending())
    {
        return Vec::new();
    }
    let mut rows = if let Some(run) = model.account.as_ref().and_then(|v| v.command.as_ref()) {
        super::command_view::active(run, width, model.tools.reduced_motion)
    } else if let Some(demo) = &model.action_demo {
        super::action_view::active(demo, width, model.tools.reduced_motion)
    } else if model.tools.active().is_some() {
        super::tool_view::active(model, width)
    } else if let Some(view) = &model.account {
        if view.notice.is_empty() {
            Vec::new()
        } else {
            lines(
                &view.notice,
                width,
                if view.failed {
                    Tone::Error
                } else {
                    Tone::Muted
                },
            )
        }
    } else {
        let state = if model.streaming() {
            "Streaming / Esc to stop"
        } else if model.status.starts_with("Simulated") {
            "Stream failed / partial response kept"
        } else if model.status.starts_with("Interrupted") {
            "Interrupted / partial response kept"
        } else if model.status.starts_with("Preview history") {
            "Preview full / restart to continue"
        } else {
            ""
        };
        if state.is_empty() {
            Vec::new()
        } else {
            lines(state, width, Tone::Muted)
        }
    };
    rows.truncate(available);
    rows
}
