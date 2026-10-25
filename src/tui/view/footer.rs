use crate::{
    context::Status,
    tui::{
        line::Line,
        state::State,
        text,
        theme::{METER_FILL, METER_TRACK, MUTED, Style, WARNING},
    },
};
use std::sync::OnceLock;

/// Panels keep the footer below them from this many rows.
const PANEL_MIN: usize = 10;
/// Cells in the context meter.
const METER_CELLS: usize = 8;
/// The meter and share turn gold this many points before auto-compaction.
const NEAR: usize = 3;
static USER_HOME: OnceLock<Option<String>> = OnceLock::new();

/// The footer: the home-relative directory on the left; the context meter and
/// share, model and effort on the right. Narrow rows fall back to the project
/// name and the model without its provider, then drop them.
pub(super) fn line(state: &State, model: &str, directory: &str, columns: usize) -> Line {
    let home = USER_HOME
        .get_or_init(|| std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).ok());
    let directory = text::clean(directory.strip_prefix(r"\\?\").unwrap_or(directory));
    let path = home_relative(&directory, home.as_deref());
    let project = directory
        .rsplit(['\\', '/'])
        .find(|part| !part.is_empty())
        .unwrap_or(&directory);
    let model = text::clean(model);
    let short = model.rsplit('/').next().unwrap_or(&model);
    let effort = &state.effort;
    let gauge = state.context.as_ref().map(gauge);
    let right = |detail: &str| {
        let mut line = gauge.clone().unwrap_or_default();
        let separator = if gauge.is_some() { " · " } else { "" };
        line.push(&format!("{separator}{detail}"), MUTED);
        line
    };
    let variants = [
        (path.as_str(), right(&format!("{model} · {effort}"))),
        (project, right(&format!("{short} · {effort}"))),
        ("", right(short)),
        ("", gauge.clone().unwrap_or_else(|| Line::new(short, MUTED))),
    ];
    let last = variants.len() - 1;
    for (index, (left, right)) in variants.into_iter().enumerate() {
        let (left_cells, right_cells) = (text::cells(left), text::cells(&right.plain()));
        let gap = if left.is_empty() { 0 } else { 2 };
        if left_cells + gap + right_cells > columns && index < last {
            continue;
        }
        let mut line = Line::new(left, MUTED);
        line.push(
            &" ".repeat(columns.saturating_sub(left_cells + right_cells)),
            MUTED,
        );
        for span in right.spans {
            line.push(&span.text, span.style);
        }
        return line.shortened(columns);
    }
    unreachable!("the last variant always renders")
}

/// `━━━━━━── 25%`: eight cells filling toward the auto-compact threshold, gold
/// when close to it. Unknown capacity leaves the track empty with `?`.
fn gauge(status: &Status) -> Line {
    let threshold = usize::from(status.auto_compact).max(1);
    let percent = status.percent();
    let near = percent.is_some_and(|percent| percent >= threshold.saturating_sub(NEAR).max(1));
    let fill: Style = if near { WARNING } else { METER_FILL };
    let filled = percent.map_or(0, |percent| {
        (percent.min(threshold) * METER_CELLS * 2 + threshold) / (threshold * 2)
    });
    let mut line = Line::default();
    line.push(&"━".repeat(filled), fill);
    line.push(&"─".repeat(METER_CELLS - filled), METER_TRACK);
    let share = match percent {
        None => "?".into(),
        Some(0) if status.tokens > 0 => "<1%".into(),
        Some(percent) => format!("{percent}%"),
    };
    line.push(&format!(" {share}"), if near { WARNING } else { MUTED });
    line
}

/// `C:\Users\me\Projects\app` → `~\Projects\app` when it lies inside `home`.
pub(super) fn home_relative(directory: &str, home: Option<&str>) -> String {
    let home = home
        .map(|home| home.trim_end_matches(['\\', '/']))
        .filter(|home| !home.is_empty());
    let split = home.and_then(|home| {
        let (prefix, rest) = (directory.get(..home.len())?, directory.get(home.len()..)?);
        let same = if cfg!(windows) {
            prefix.eq_ignore_ascii_case(home)
        } else {
            prefix == home
        };
        (same && (rest.is_empty() || rest.starts_with(['\\', '/']))).then_some(rest)
    });
    split.map_or_else(|| directory.into(), |rest| format!("~{rest}"))
}

/// Rows a panel of `height` gives to the footer: one when the engine status
/// is known and the panel keeps its input, selection and controls.
pub(super) fn panel_rows(state: &State, height: usize) -> usize {
    usize::from(state.context.is_some() && state.width >= 7 && height >= PANEL_MIN)
}
