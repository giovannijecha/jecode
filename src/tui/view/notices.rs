//! Notices by lifetime: a quiet line for results, a banner for warnings and
//! errors, and a label for progress beside the composer's scanner.
use super::panel;
use crate::tui::{
    feedback::Feedback,
    line::Line,
    state::Kind,
    text,
    theme::{BAD, MUTED, STRONG, Style, USER_SURFACE, WARNING},
};
use std::time::Instant;

/// Muted `Subject · detail`, or a banner on the panel surface
/// with a stripe in the level colour. Rows wrap under the subject; the action
/// keycap sits at the right of the last row, or on a row of its own.
pub(super) fn lines(notice: &Feedback, columns: usize) -> Vec<Line> {
    let banner = matches!(notice.kind, Kind::Warning | Kind::Error);
    let (lead, style) = match notice.kind {
        Kind::Error => ("▌ ", BAD),
        Kind::Warning => ("▌ ", WARNING),
        _ => ("", MUTED),
    };
    // A banner keeps one cell of its surface at the right edge.
    let width = columns.saturating_sub(usize::from(banner)).max(1);
    let mut rows = hang(Line::new(lead, style), body(&notice.text, style), width);
    if let Some((key, label)) = notice.action {
        let mut action = panel::keycap(key);
        action.push(&format!(" {label}"), MUTED);
        let last = rows.pop().unwrap_or_default();
        let (used, room) = (panel::cells(&last), panel::cells(&action));
        if used + 2 + room <= width {
            rows.push(panel::spread(last, action, width, None));
        } else {
            rows.push(last);
            rows.push(action.indent("  ", MUTED).shortened(width));
        }
    }
    if banner {
        rows.into_iter().map(|row| row.on(USER_SURFACE)).collect()
    } else {
        rows
    }
}

/// Progress beside the scanner: the subject strong, or gold while a request
/// waits to retry, then the countdown to the next attempt.
pub(super) fn progress(notice: &Feedback) -> Line {
    label(notice, &notice.text)
}

/// Shorter forms for a narrow rule: trailing ` · ` details drop one at a
/// time, down to the subject, and the countdown always stays.
pub(super) fn progress_forms(notice: &Feedback) -> Vec<Line> {
    let text = notice.text.as_str();
    let mut ends: Vec<_> = text.match_indices(" · ").map(|(at, _)| at).collect();
    ends.reverse();
    ends.retain(|&at| at >= subject(text).len());
    ends.into_iter()
        .map(|at| label(notice, &text[..at]))
        .collect()
}

fn label(notice: &Feedback, text: &str) -> Line {
    let subject = if matches!(notice.kind, Kind::Warning) {
        WARNING
    } else {
        STRONG
    };
    let mut line = body(text, subject);
    if let Some(until) = notice.until {
        let left = until.saturating_duration_since(Instant::now());
        if !left.is_zero() {
            let seconds = left.as_secs() + u64::from(left.subsec_nanos() > 0);
            line.push(&format!(" in {seconds}s"), MUTED);
        }
    }
    line
}

/// `Subject · detail`, where the subject ends at the first ` · ` or `: `.
/// After a colon, `old → new` reads as a change without the colon: the new
/// value strong. Any other detail keeps its separator.
fn body(value: &str, style: Style) -> Line {
    let value = text::clean(value);
    let at = subject(&value).len();
    if at == value.len() {
        return Line::new(&value, style);
    }
    let separator = if value[at..].starts_with(": ") {
        ": "
    } else {
        " · "
    };
    let rest = &value[at + separator.len()..];
    let mut line = Line::new(&value[..at], style);
    let (first, tail) = rest.split_once(" · ").unwrap_or((rest, ""));
    match first.split_once(" → ").filter(|_| separator == ": ") {
        Some((from, to)) => {
            line.push(&format!(" {from} → "), MUTED);
            line.push(to, STRONG);
            if !tail.is_empty() {
                line.push(&format!(" · {tail}"), MUTED);
            }
        }
        None => line.push(&format!("{separator}{rest}"), MUTED),
    }
    line
}

fn subject(value: &str) -> &str {
    let at = [value.find(" · "), value.find(": ")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(value.len());
    &value[..at]
}

/// `body` wrapped after `lead`, continuation rows indented beneath it.
fn hang(lead: Line, body: Line, width: usize) -> Vec<Line> {
    let indent = panel::cells(&lead);
    if width <= indent + 1 {
        return body.wrap(width, true);
    }
    body.wrap(width - indent, true)
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            if index == 0 {
                lead.clone().append(row)
            } else {
                row.indent(&" ".repeat(indent), MUTED)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
