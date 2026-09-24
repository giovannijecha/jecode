//! Runtime state belongs to the transient transcript tail above the input rule.
use super::{
    model::Model,
    style::{Row, Tone, lines},
};

pub(super) fn rows(model: &Model, width: usize, available: usize) -> Vec<Row> {
    let mut rows = if let Some(run) = model.account.as_ref().and_then(|v| v.command.as_ref()) {
        super::command_view::active(run, width, model.tools.reduced_motion)
    } else if let Some(demo) = &model.action_demo {
        super::action_view::active(demo, width, model.tools.reduced_motion)
    } else if model.tools.active().is_some() {
        super::tool_view::active(model, width)
    } else if let Some(view) = &model.account {
        if view.notice.is_empty() {
            Vec::new()
        } else if let Some(label) = model_label(model) {
            vec![super::tool_view::indicator(
                model.status_spinner.marker(model.tools.reduced_motion),
                label,
                width,
            )]
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
        let state = if model_label(model).is_some() {
            "Streaming"
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
        } else if model_label(model).is_some() {
            vec![super::tool_view::indicator(
                model.status_spinner.marker(model.tools.reduced_motion),
                state,
                width,
            )]
        } else {
            lines(state, width, Tone::Muted)
        }
    };
    rows.truncate(available);
    rows
}

/// Only ordinary model generation gets a generic marker. A tool or command
/// owns the same status position while it is active.
pub(super) fn model_label(model: &Model) -> Option<&str> {
    if model.action_demo.is_some()
        || model.tools.active().is_some()
        || model
            .account
            .as_ref()
            .is_some_and(|view| view.command.is_some() || view.edit.is_some())
    {
        return None;
    }
    if let Some(view) = &model.account {
        if !view.generating() {
            return None;
        }
        return match view.notice.as_str() {
            "Waiting for model" | "Thinking" | "Streaming" | "Compacting context" => {
                Some(&view.notice)
            }
            _ => None,
        };
    }
    model.streaming().then_some("Streaming")
}
