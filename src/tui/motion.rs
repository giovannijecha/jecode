//! The one motion clock: the working caret blinks and a single shimmer band
//! runs through the status row and running badges at the same speed.
use super::{
    line::Line,
    text,
    theme::{ACCENT, LIT, S_BASE, S0, S1, S2, Style},
};
use std::time::Duration;

/// One frame of motion.
pub const FRAME: Duration = Duration::from_millis(100);

/// The current frame, counted from the first call in this process. Tests
/// hold it at zero unless they set it, so plain text stays deterministic.
#[cfg(not(test))]
pub fn frame() -> usize {
    static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let elapsed = EPOCH.get_or_init(std::time::Instant::now).elapsed();
    (elapsed.as_millis() / FRAME.as_millis()) as usize
}

#[cfg(test)]
thread_local! {
    static TEST_FRAME: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub fn frame() -> usize {
    TEST_FRAME.with(std::cell::Cell::get)
}

#[cfg(test)]
pub fn set_frame(frame: usize) {
    TEST_FRAME.with(|cell| cell.set(frame));
}

/// `>` fixed, `_` on for 400 ms and off for 400 ms. `tone` recolours both
/// glyphs, as retries do in `WARNING`.
pub fn caret(frame: usize, tone: Option<Style>) -> Line {
    let mut line = Line::new(">", tone.unwrap_or(ACCENT).bold());
    let lit = (frame / 4).is_multiple_of(2);
    line.push(if lit { "_" } else { " " }, tone.unwrap_or(LIT).bold());
    line
}

/// `label` with a five-cell band `S0 S1 S2 S1 S0` over `S_BASE`, one cell
/// per frame; the band enters and leaves past both ends.
pub fn shimmer(label: &str, frame: usize, bold: bool) -> Line {
    let cells = text::cells(label);
    let centre = (frame % (cells + 8)) as isize - 4;
    let mut line = Line::default();
    let mut at = 0isize;
    for glyph in text::glyphs(label) {
        let style = match (at - centre).unsigned_abs() {
            0 => S2,
            1 => S1,
            2 => S0,
            _ => S_BASE,
        };
        line.push(glyph.text, if bold { style.bold() } else { style });
        at += glyph.cells as isize;
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_caret_blinks_400_ms_on_and_400_ms_off() {
        let frames: Vec<_> = (0..9).map(|frame| caret(frame, None).plain()).collect();
        assert_eq!(frames[..4], [">_", ">_", ">_", ">_"]);
        assert_eq!(frames[4..8], ["> ", "> ", "> ", "> "]);
        assert_eq!(frames[8], ">_");
        assert_eq!(caret(0, None).spans[0].style, ACCENT.bold());
        assert_eq!(caret(0, None).spans[1].style, LIT.bold());
        let warning = caret(0, Some(crate::tui::theme::WARNING));
        assert!(
            warning
                .spans
                .iter()
                .all(|span| span.style == crate::tui::theme::WARNING.bold())
        );
    }

    #[test]
    fn the_shimmer_band_moves_one_cell_per_frame_and_wraps() {
        // One style per glyph; equal neighbours share a span.
        let styles = |frame| {
            shimmer("Reading", frame, false)
                .spans
                .iter()
                .flat_map(|span| span.text.chars().map(move |_| span.style))
                .collect::<Vec<_>>()
        };
        // The centre starts four cells before the label.
        assert!(styles(0).iter().all(|style| *style == S_BASE));
        assert_eq!(styles(2)[0], S0);
        assert_eq!(styles(4)[..3], [S2, S1, S0]);
        assert_eq!(styles(7)[1..6], [S0, S1, S2, S1, S0]);
        assert_eq!(styles(4 + 7 + 8), styles(4));
        assert_eq!(shimmer("Reading", 4, false).plain(), "Reading");
        // A wide glyph counts its cells: `a` sits at cell 2.
        assert_eq!(shimmer("界a", 4, false).spans[1].style, S0);
    }
}
