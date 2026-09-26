//! Tool steps as one linear tree. Each step is a single line — branch, state
//! mark, verb, subject, summary, elapsed — with its detail indented under the
//! verb and cut to a few lines so a long command never floods the transcript.
//! `expanded` (Ctrl+O) shows every detail line instead; the cut marker names
//! the key that lifts it.
//!
//! ```text
//!  ├─ ✓ Read src/terminal/render.rs · 124 lines       0.1s
//!  └─ ⠹ Run cargo test                                1.4s
//!       … 12 earlier lines
//!       test result: ok. 61 passed
//! ```
use super::activity_view::elapsed;
use super::block::MARGIN;
use super::glyph::Glyphs;
use super::model::{Detail, Status, Tool};
use super::style::{Row, Tone};
use super::text;

/// Output keeps its tail (results come last); diffs keep their head.
const OUTPUT_TAIL: usize = 5;
const DIFF_HEAD: usize = 8;

pub fn rows(
    tools: &[Tool],
    width: usize,
    glyph: &Glyphs,
    spinner: &str,
    expanded: bool,
) -> Vec<Row> {
    let mut out = Vec::new();
    for (index, tool) in tools.iter().enumerate() {
        let last = index + 1 == tools.len();
        out.push(step(tool, last, width, glyph, spinner));
        let stem = if last { " " } else { glyph.stem };
        detail(tool, stem, width, glyph, expanded, &mut out);
    }
    out
}

/// The state mark shared by tool steps and command receipts.
pub fn mark<'a>(status: Status, glyph: &'a Glyphs, spinner: &'a str) -> (&'a str, Tone) {
    match status {
        Status::Running => (spinner, Tone::Accent),
        Status::Done => (glyph.ok, Tone::Success),
        Status::Warned => (glyph.warn, Tone::Warning),
        Status::Failed => (glyph.fail, Tone::Error),
    }
}

fn step(tool: &Tool, last: bool, width: usize, glyph: &Glyphs, spinner: &str) -> Row {
    let (mark, tone) = mark(tool.status, glyph, spinner);
    let mut row = Row::new(MARGIN, Tone::Text);
    row.push(if last { glyph.last } else { glyph.branch }, Tone::Muted)
        .push(" ", Tone::Text)
        .push(mark, tone)
        .push(" ", Tone::Text)
        .push(&tool.verb, Tone::Heading)
        .push(" ", Tone::Text);
    let time = if tool.elapsed_ms == u64::MAX {
        String::new()
    } else {
        elapsed(tool.elapsed_ms, false)
    };
    let summary = match tool.summary.is_empty() {
        true => String::new(),
        false => format!(" {} {}", glyph.dot, tool.summary),
    };
    // Right side: two cells of air, the time, and the closing margin.
    let room = width.saturating_sub(text::width(&row.text) + text::width(&time) + 2 + MARGIN.len());
    let subject_room = if room >= 24 {
        (room * 2 / 3).min(text::width(&tool.subject))
    } else {
        room
    };
    let subject = text::clip(&tool.subject, subject_room, glyph.ellipsis);
    let summary_room = room.saturating_sub(text::width(&subject));
    let summary = if summary_room >= 4 {
        text::clip(&summary, summary_room, glyph.ellipsis)
    } else {
        String::new()
    };
    row.push(&subject, Tone::Text).push(&summary, Tone::Muted);
    let gap = width.saturating_sub(text::width(&row.text) + text::width(&time) + MARGIN.len());
    row.push(&" ".repeat(gap), Tone::Text)
        .push(&time, Tone::Muted);
    row
}

fn detail(
    tool: &Tool,
    stem: &str,
    width: usize,
    glyph: &Glyphs,
    expanded: bool,
    out: &mut Vec<Row>,
) {
    // Detail text starts under the verb: margin, stem, four cells.
    let lead = MARGIN.len() + text::width(stem) + 4;
    let room = width.saturating_sub(lead + MARGIN.len());
    let line_row = |line: &str, tone: Tone, band: bool| {
        let mut row = Row::new(MARGIN, Tone::Text);
        row.push(stem, Tone::Muted).push("    ", Tone::Text);
        let body = text::clip(line, room, glyph.ellipsis);
        let body = if band { text::pad(&body, room) } else { body };
        row.push(&body, tone);
        row
    };
    let time_width = if tool.elapsed_ms == u64::MAX {
        0
    } else {
        text::width(&elapsed(tool.elapsed_ms, false))
    };
    let full_width = MARGIN.len()
        + text::width(glyph.last)
        + 3
        + text::width(&tool.verb)
        + 1
        + text::width(&tool.subject)
        + text::width(&format!(" {} {}", glyph.dot, tool.summary))
        + 2
        + time_width
        + MARGIN.len();
    if matches!(tool.status, Status::Warned | Status::Failed)
        && !tool.summary.is_empty()
        && full_width > width
    {
        out.push(line_row(
            &tool.summary,
            if tool.status == Status::Failed {
                Tone::Error
            } else {
                Tone::Warning
            },
            false,
        ));
    }
    let more = |count: usize, word: &str| {
        let label = format!("{} {count} {word} lines", glyph.ellipsis);
        // The key hint goes first when the row is tight: never a clipped key.
        let hint = format!("{label} {} ctrl+o", glyph.dot);
        let label = if text::width(&hint) <= room {
            hint
        } else {
            label
        };
        line_row(&label, Tone::Muted, false)
    };
    match &tool.detail {
        Detail::Output(output) => {
            let lines: Vec<&str> = output.trim_end().lines().collect();
            let keep = if expanded { lines.len() } else { OUTPUT_TAIL };
            let hidden = lines.len().saturating_sub(keep);
            if hidden > 0 {
                out.push(more(hidden, "earlier"));
            }
            let tone = match tool.status {
                Status::Failed => Tone::Error,
                _ => Tone::Muted,
            };
            for line in &lines[hidden..] {
                out.push(line_row(line, tone, false));
            }
        }
        Detail::Diff(diff) => {
            let lines: Vec<&str> = diff.trim_end().lines().collect();
            let keep = if expanded { lines.len() } else { DIFF_HEAD };
            for line in lines.iter().take(keep) {
                let (tone, band) = match line.as_bytes().first() {
                    Some(b'+') => (Tone::Added, true),
                    Some(b'-') => (Tone::Removed, true),
                    _ if line.starts_with("@@") => (Tone::Accent, false),
                    _ => (Tone::Muted, false),
                };
                out.push(line_row(line, tone, band));
            }
            if lines.len() > keep {
                out.push(more(lines.len() - keep, "more"));
            }
        }
    }
}

#[cfg(test)]
#[path = "tool_view_tests.rs"]
mod tests;
