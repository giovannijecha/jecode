use crate::{
    context::Status,
    tui::{
        line::Line,
        state::State,
        text,
        theme::{DIM, METER_FILL, METER_TRACK, MUTED, PILL_SURFACE, Style, WARNING},
    },
};
use std::sync::OnceLock;

/// Cells in the context meter.
const METER_CELLS: usize = 8;
/// The meter and share turn gold this many points before auto-compaction.
const NEAR: usize = 3;
static USER_HOME: OnceLock<Option<String>> = OnceLock::new();

/// The footer: the home-relative path in `dim` on the left; on the right the
/// model id and effort on a chip, two cells, then the context gauge. Narrow
/// rows drop the provider, fall back to the project name, then drop effort,
/// the path and the chip.
pub(super) fn line(state: &State, model: &str, directory: &str, columns: usize) -> Line {
    let path = path(directory);
    let directory = text::clean(directory.strip_prefix(r"\\?\").unwrap_or(directory));
    let project = directory
        .rsplit(['\\', '/'])
        .find(|part| !part.is_empty())
        .unwrap_or(&directory);
    let model = text::clean(model);
    let short = model.rsplit('/').next().unwrap_or(&model);
    let effort = &state.effort;
    let gauge = state.context.as_ref().map(gauge);
    let right = |chip: Option<&str>| {
        let mut line = Line::default();
        if let Some(chip) = chip {
            line.push_on(&format!(" {chip} "), MUTED, PILL_SURFACE);
        }
        match &gauge {
            Some(gauge) if chip.is_some() => {
                line.push("  ", MUTED);
                line.append(gauge.clone())
            }
            Some(gauge) => gauge.clone(),
            None => line,
        }
    };
    let full = format!("{model} · {effort}");
    let detailed = format!("{short} · {effort}");
    let variants = [
        (path.as_str(), right(Some(&full))),
        (path.as_str(), right(Some(&detailed))),
        (project, right(Some(&detailed))),
        (project, right(Some(short))),
        ("", right(Some(short))),
        ("", gauge.clone().unwrap_or_else(|| right(Some(short)))),
    ];
    let last = variants.len() - 1;
    for (index, (left, right)) in variants.into_iter().enumerate() {
        let (left_cells, right_cells) = (text::cells(left), text::cells(&right.plain()));
        let gap = if left.is_empty() { 0 } else { 2 };
        if left_cells + gap + right_cells > columns && index < last {
            continue;
        }
        let mut line = Line::new(left, DIM);
        line.push(
            &" ".repeat(columns.saturating_sub(left_cells + right_cells)),
            DIM,
        );
        return line.append(right).shortened(columns);
    }
    unreachable!("the last variant always renders")
}

/// `▰▰▰▰▱▱▱▱ 52%`: eight cells filling toward the auto-compact threshold,
/// `warning` within three points of it. Unknown capacity leaves the track
/// empty with `?`.
fn gauge(status: &Status) -> Line {
    let threshold = usize::from(status.auto_compact).max(1);
    let percent = status.percent();
    let near = percent.is_some_and(|percent| percent >= threshold.saturating_sub(NEAR).max(1));
    let fill: Style = if near { WARNING } else { METER_FILL };
    let filled = percent.map_or(0, |percent| {
        (percent.min(threshold) * METER_CELLS * 2 + threshold) / (threshold * 2)
    });
    let mut line = Line::default();
    line.push(&"▰".repeat(filled), fill);
    line.push(&"▱".repeat(METER_CELLS - filled), METER_TRACK);
    let share = match percent {
        None => "?".into(),
        Some(0) if status.tokens > 0 => "<1%".into(),
        Some(percent) => format!("{percent}%"),
    };
    line.push(&format!(" {share}"), if near { WARNING } else { MUTED });
    line
}

/// The working directory as shown, relative to the user's home.
pub(super) fn path(directory: &str) -> String {
    let home = USER_HOME
        .get_or_init(|| std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).ok());
    let directory = text::clean(directory.strip_prefix(r"\\?\").unwrap_or(directory));
    home_relative(&directory, home.as_deref())
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
