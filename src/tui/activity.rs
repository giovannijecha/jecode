use super::{
    line::Line,
    theme::{SCANNER_LIT, SCANNER_REST, TOOL_DIM},
};
use std::time::{Duration, Instant};

pub struct Activity {
    pub started: Instant,
    pub tools: usize,
    pub stopping: bool,
}
impl Activity {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            tools: 0,
            stopping: false,
        }
    }
    /// The scanner and the whole turn's elapsed time, as the composer rule shows them.
    pub fn line(&self, time: bool) -> Line {
        Self::at(self.started.elapsed(), time)
    }
    fn at(elapsed: Duration, time: bool) -> Line {
        let mut line = indicator((elapsed.as_millis() / 120) as usize);
        if time {
            line.push(&format!(" {}", duration(elapsed)), TOOL_DIM);
        }
        line
    }
}

/// Two lit blocks sweeping across six cells and back.
pub(super) fn indicator(frame: usize) -> Line {
    const CELLS: usize = 6;
    let lit = 4 - (frame % 8).abs_diff(4);
    let mut line = Line::default();
    for cell in 0..CELLS {
        if cell == lit || cell == lit + 1 {
            line.push("▰", SCANNER_LIT);
        } else {
            line.push("▱", SCANNER_REST);
        }
    }
    line
}

pub fn duration(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds < 60 {
        format!("{:.1}s", elapsed.as_secs_f64())
    } else if seconds < 3600 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {:02}m", seconds / 3600, seconds / 60 % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scanner_sweeps_two_blocks_across_six_cells() {
        let frames: Vec<_> = (0..8).map(|frame| indicator(frame).plain()).collect();
        assert_eq!(frames[0], "▰▰▱▱▱▱");
        assert_eq!(frames[4], "▱▱▱▱▰▰");
        assert_eq!(frames[7], "▱▰▰▱▱▱");
        assert_eq!(indicator(8).plain(), frames[0]);
    }

    #[test]
    fn duration_moves_from_tenths_to_minutes_to_hours() {
        assert_eq!(duration(Duration::from_millis(3200)), "3.2s");
        assert_eq!(duration(Duration::from_secs(64)), "1m 04s");
        assert_eq!(duration(Duration::from_secs(70 * 60 + 52)), "1h 10m");
        assert_eq!(duration(Duration::from_secs(3600 + 5 * 60)), "1h 05m");
        assert_eq!(
            Activity::at(Duration::from_millis(3200), true).plain(),
            "▱▱▰▰▱▱ 3.2s"
        );
        assert_eq!(Activity::at(Duration::ZERO, false).plain(), "▰▰▱▱▱▱");
    }
}
