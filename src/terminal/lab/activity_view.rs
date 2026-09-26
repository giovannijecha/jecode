//! The working row above the composer: braille rectangle, phase label with a
//! light shimmer, elapsed time and token count, and the cancel hint. A
//! standing notice, when there is one, sits just above it.
use super::block::MARGIN;
use super::caps::Caps;
use super::glyph::Glyphs;
use super::model::{Notice, Status};
use super::motion;
use super::style::{Row, Tone};
use super::text;
use super::unicode::boundaries;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Waiting,
    Thinking,
    Streaming,
    /// Tools are running.
    Working,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Self::Waiting => "Waiting for model",
            Self::Thinking => "Thinking",
            Self::Streaming => "Streaming",
            Self::Working => "Working",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Activity {
    pub phase: Phase,
    pub elapsed_ms: u64,
    pub tokens: usize,
}

const HINT: &str = "Esc stops";

pub fn row(activity: &Activity, width: usize, caps: &Caps, glyph: &Glyphs, now_ms: u64) -> Row {
    let mut row = Row::new(MARGIN, Tone::Text);
    row.push(&motion::spinner(now_ms, caps), Tone::Accent)
        .push(" ", Tone::Text);
    let dot = format!(" {} ", glyph.dot);
    let mut facts = format!("{dot}{}", elapsed(activity.elapsed_ms, caps.reduced_motion));
    if activity.tokens > 0 {
        facts.push_str(&format!("{dot}{} tok", activity.tokens));
    }
    let available = width.saturating_sub(text::width(&row.text) + MARGIN.len());
    if text::width(&facts) >= available {
        facts = text::clip(&facts, available.saturating_sub(1), glyph.ellipsis);
    }
    let label_room = available.saturating_sub(text::width(&facts));
    let label = text::clip(activity.phase.label(), label_room, glyph.ellipsis);
    let stops = boundaries(&label);
    match motion::shimmer(now_ms, stops.len() - 1, caps) {
        Some(window) => {
            let (from, to) = (stops[window.start], stops[window.end]);
            row.push(&label[..from], Tone::Heading)
                .push(&label[from..to], Tone::Accent)
                .push(&label[to..], Tone::Heading);
        }
        None => {
            row.push(&label, Tone::Heading);
        }
    }
    row.push(&facts, Tone::Muted);
    let used = text::width(&row.text);
    let room = width.saturating_sub(used + MARGIN.len());
    if room >= text::width(HINT) + 2 {
        row.push(&" ".repeat(room - text::width(HINT)), Tone::Text)
            .push(HINT, Tone::Muted);
    }
    row
}

/// The notice row: state mark, headline (red when failed), muted detail.
pub fn notice(notice: &Notice, width: usize, glyph: &Glyphs) -> Row {
    let (mark, tone) = super::tool_view::mark(notice.status, glyph, "");
    let mut row = Row::new(MARGIN, Tone::Text);
    row.push(mark, tone).push(" ", Tone::Text);
    let room = width.saturating_sub(text::width(&row.text) + MARGIN.len());
    let body = text::clip(&notice.text, room, glyph.ellipsis);
    let (head, detail) = match body.split_once(" · ") {
        Some((head, _)) => (head, &body[head.len()..]),
        None => (body.as_str(), ""),
    };
    let head_tone = match notice.status {
        Status::Failed => Tone::Error,
        _ => Tone::Text,
    };
    row.push(head, head_tone).push(detail, Tone::Muted);
    row
}

/// `3.2s`, `1m 04s`; whole seconds under reduced motion so the row only
/// changes once a second.
pub fn elapsed(ms: u64, reduced: bool) -> String {
    let seconds = ms / 1000;
    if seconds >= 60 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else if reduced {
        format!("{seconds}s")
    } else {
        format!("{seconds}.{}s", ms % 1000 / 100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::lab::caps::ColorDepth;
    use crate::terminal::lab::glyph::UNICODE;

    const CAPS: Caps = Caps {
        color: ColorDepth::TrueColor,
        ascii: false,
        reduced_motion: true,
    };

    #[test]
    fn notice_marks_the_state_and_mutes_the_detail() {
        let failed = Notice {
            status: Status::Failed,
            text: "Provider unreachable · retrying in 3s".into(),
        };
        let row = notice(&failed, 60, &UNICODE);
        assert_eq!(row.text, " ✗ Provider unreachable · retrying in 3s");
        assert!(row.spans.contains(&(5..25, Tone::Error)));
        assert_eq!(row.spans.last().map(|span| span.1), Some(Tone::Muted));
        let warned = Notice {
            status: Status::Warned,
            ..failed
        };
        assert!(notice(&warned, 12, &UNICODE).text.starts_with(" ! Provid"));
    }

    #[test]
    fn elapsed_formats() {
        assert_eq!(elapsed(3_250, false), "3.2s");
        assert_eq!(elapsed(3_250, true), "3s");
        assert_eq!(elapsed(64_000, false), "1m 04s");
    }

    #[test]
    fn row_fills_the_width_with_a_right_aligned_hint() {
        let activity = Activity {
            phase: Phase::Streaming,
            elapsed_ms: 3_200,
            tokens: 412,
        };
        let row = row(&activity, 50, &CAPS, &UNICODE, 0);
        assert_eq!(
            row.text.trim_end(),
            " ⣿ Streaming · 3s · 412 tok             Esc stops"
        );
        assert_eq!(text::width(&row.text), 49);
    }

    #[test]
    fn hint_drops_when_narrow() {
        let activity = Activity {
            phase: Phase::Thinking,
            elapsed_ms: 0,
            tokens: 0,
        };
        assert_eq!(
            row(&activity, 22, &CAPS, &UNICODE, 0).text,
            " ⣿ Thinking · 0s"
        );
    }
}
