use super::fitted;
use crate::{
    context::Status,
    tui::{line::Line, state::State, text},
};

/// Panels keep the footer below them from this many rows.
const PANEL_MIN: usize = 10;
/// Directory cells kept before the indicator abbreviates.
const DIRECTORY_MIN: usize = 12;
/// Below this many cells a shortened directory is omitted, not left as a lone ellipsis.
const DIRECTORY_FLOOR: usize = 6;

/// The footer: directory on the left; context, model and effort on the right.
pub(super) fn line(state: &State, model: &str, directory: &str, columns: usize) -> Line {
    let directory = directory.strip_prefix(r"\\?\").unwrap_or(directory);
    let model = format!("{model} · {}", state.effort);
    let right = match &state.context {
        Some(status) => {
            let room = text::cells(&text::clean(directory)).min(DIRECTORY_MIN) + 2;
            let variants = indicators(status);
            // The indicator leads, so narrow rows shorten the model before it.
            variants
                .iter()
                .map(|indicator| format!("{indicator} · {model}"))
                .find(|right| text::cells(right) + room <= columns)
                .unwrap_or_else(|| format!("{} · {model}", variants[variants.len() - 1]))
        }
        None => model,
    };
    let budget = columns.saturating_sub(text::cells(&right) + 2);
    let directory = if budget < text::cells(&text::clean(directory)).min(DIRECTORY_FLOOR) {
        ""
    } else {
        directory
    };
    fitted(directory, &right, columns)
}

/// Indicator labels from fullest to shortest. Unknown capacity never shows a percentage.
pub(super) fn indicators(status: &Status) -> [String; 3] {
    let auto = format!("Auto {}%", status.auto_compact);
    match status.percent() {
        Some(percent) => {
            let value = if percent == 0 && status.tokens > 0 {
                "<1%".into()
            } else {
                format!("≈{percent}%")
            };
            [
                format!("Context {value} · {auto}"),
                format!("Context {value}"),
                value,
            ]
        }
        None => [
            format!("Context unknown · {auto}"),
            "Context unknown".into(),
            "Context ?".into(),
        ],
    }
}

/// Rows a panel of `height` gives to the footer: one when the engine status
/// is known and the panel keeps its input, selection and controls.
pub(super) fn panel_rows(state: &State, height: usize) -> usize {
    usize::from(state.context.is_some() && state.width >= 7 && height >= PANEL_MIN)
}
