//! Animation as pure functions of a clock. Nothing here sleeps or schedules;
//! reduced motion freezes every effect into a still frame.
use super::caps::Caps;
use std::ops::Range;

/// One animation step. Spinner, shimmer and caret all advance on this beat.
pub const FRAME_MS: u64 = 80;
const CARET_MS: u64 = 500;

/// The full 2×4 dot rectangle of one braille cell, walked clockwise; half
/// the border is lit at any time. Chosen by eye over 2×3 subsets, which sat
/// visibly high or low next to the label.
const PERIMETER: [(usize, usize); 8] = [
    (0, 0),
    (1, 0),
    (1, 1),
    (1, 2),
    (1, 3),
    (0, 3),
    (0, 2),
    (0, 1),
];
const TRAIL: usize = 4;
const ASCII_SPIN: [&str; 4] = ["-", "\\", "|", "/"];

/// The working indicator, one cell wide: the braille rectangle, or a
/// classic `-\|/` spinner in ASCII.
pub fn spinner(now_ms: u64, caps: &Caps) -> String {
    let step = (now_ms / FRAME_MS) as usize;
    match (caps.ascii, caps.reduced_motion) {
        (true, true) => "*".into(),
        (true, false) => ASCII_SPIN[step % ASCII_SPIN.len()].into(),
        (false, true) => "⣿".into(),
        (false, false) => braille(step % PERIMETER.len()).into(),
    }
}

fn braille(step: usize) -> char {
    let bits = (0..TRAIL)
        .map(|offset| PERIMETER[(step + offset) % PERIMETER.len()])
        .fold(0, |bits, (x, y)| bits | dot(x, y));
    char::from_u32(0x2800 + bits).unwrap_or(' ')
}

/// Braille bit for a dot in the cell's 2×4 grid (Unicode dot numbering).
fn dot(column: usize, row: usize) -> u32 {
    match (column, row) {
        (0, 3) => 0x40,
        (_, 3) => 0x80,
        (0, row) => 1 << row,
        (_, row) => 8 << row,
    }
}

/// Streaming caret; blinks at 1 Hz, steady under reduced motion.
pub fn caret(now_ms: u64, caps: &Caps) -> Option<&'static str> {
    let on = caps.reduced_motion || (now_ms / CARET_MS).is_multiple_of(2);
    on.then_some(if caps.ascii { "_" } else { "▍" })
}

/// Cluster window of the highlight sweeping across a label of `len`
/// clusters, with a pause between passes. `None` means no highlight now.
pub fn shimmer(now_ms: u64, len: usize, caps: &Caps) -> Option<Range<usize>> {
    const WIDTH: usize = 3;
    const PAUSE: usize = 8;
    if caps.reduced_motion || len == 0 {
        return None;
    }
    let head = (now_ms / FRAME_MS) as usize % (len + WIDTH + PAUSE);
    let range = head.saturating_sub(WIDTH)..head.min(len);
    (range.start < range.end).then_some(range)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::lab::caps::ColorDepth;

    const CAPS: Caps = Caps {
        color: ColorDepth::TrueColor,
        ascii: false,
        reduced_motion: false,
    };

    #[test]
    fn braille_trail_circles_the_rectangle() {
        let frames: String = (0..8).map(|step| spinner(step * FRAME_MS, &CAPS)).collect();
        assert_eq!(frames, "⠹⢸⣰⣤⣆⡇⠏⠛");
        assert_eq!(spinner(8 * FRAME_MS, &CAPS), "⠹");
    }

    #[test]
    fn reduced_motion_freezes_everything() {
        let still = Caps {
            reduced_motion: true,
            ..CAPS
        };
        assert_eq!(spinner(0, &still), spinner(999, &still));
        assert_eq!(caret(CARET_MS, &still), Some("▍"));
        assert_eq!(shimmer(400, 10, &still), None);
    }

    #[test]
    fn ascii_spinner_and_caret() {
        let ascii = Caps {
            ascii: true,
            ..CAPS
        };
        assert_eq!(spinner(FRAME_MS, &ascii), "\\");
        assert_eq!(caret(0, &ascii), Some("_"));
        assert_eq!(caret(CARET_MS, &ascii), None);
    }

    #[test]
    fn shimmer_sweeps_then_pauses() {
        assert_eq!(shimmer(FRAME_MS, 5, &CAPS), Some(0..1));
        assert_eq!(shimmer(4 * FRAME_MS, 5, &CAPS), Some(1..4));
        assert_eq!(shimmer(9 * FRAME_MS, 5, &CAPS), None);
    }
}
