use super::{
    line::Line,
    text,
    theme::{EMPHASIS, TOOL_DIM, WAVE_SHADES},
};
use std::time::{Duration, Instant};

pub struct Activity {
    pub started: Instant,
    pub phase_started: Instant,
    pub label: &'static str,
    pub tools: usize,
    pub stopping: bool,
}
impl Activity {
    pub fn new() -> Self {
        let started = Instant::now();
        Self {
            started,
            phase_started: started,
            label: "Preparing",
            tools: 0,
            stopping: false,
        }
    }
    pub fn set_label(&mut self, label: &'static str) {
        if self.label != label {
            self.label = label;
            self.phase_started = Instant::now();
        }
    }
    pub fn line(&self, columns: usize, hint: &str) -> Line {
        self.at(self.phase_started.elapsed(), columns, hint)
    }
    /// One sentence: the current phase and its duration, then a key hint when it
    /// fits whole. Progress such as a reconnect is a notice above this row.
    fn at(&self, elapsed: Duration, columns: usize, hint: &str) -> Line {
        let mut line = indicator((elapsed.as_millis() / 120) as usize);
        line.push("  ", TOOL_DIM);
        line.push(
            if self.label == "Waiting" {
                "Waiting for the model"
            } else {
                self.label
            },
            EMPHASIS.bold(),
        );
        line.push(&format!("  ·  {}", duration(elapsed)), TOOL_DIM);
        if !hint.is_empty() && text::cells(&line.plain()) + 3 + text::cells(hint) <= columns {
            line.push(
                &" ".repeat(columns - text::cells(&line.plain()) - text::cells(hint)),
                TOOL_DIM,
            );
            line.push(hint, TOOL_DIM);
        }
        line.shortened(columns)
    }
}

pub(super) fn indicator(frame: usize) -> Line {
    // A constant glyph and weight keep all four dots on the same baseline.
    const WAVE: [[usize; 4]; 12] = [
        [3, 1, 0, 0],
        [2, 2, 0, 0],
        [1, 3, 1, 0],
        [0, 2, 2, 0],
        [0, 1, 3, 1],
        [0, 0, 2, 2],
        [0, 0, 1, 3],
        [0, 0, 2, 2],
        [0, 1, 3, 1],
        [0, 2, 2, 0],
        [1, 3, 1, 0],
        [2, 2, 0, 0],
    ];
    let mut line = Line::default();
    for shade in WAVE[frame % WAVE.len()] {
        line.push("•", WAVE_SHADES[shade]);
    }
    line
}

pub fn duration(elapsed: Duration) -> String {
    if elapsed.as_secs() < 60 {
        format!("{:.1}s", elapsed.as_secs_f64())
    } else {
        format!("{}m {:02}s", elapsed.as_secs() / 60, elapsed.as_secs() % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn animation_has_fixed_positions_and_duration_spans_the_current_phase() {
        let activity = Activity::new();
        let at = |elapsed, columns| activity.at(elapsed, columns, "esc to stop").plain();
        let wide = at(Duration::from_millis(3200), 80);
        assert!(wide.starts_with("••••  Preparing  ·  3.2s"));
        assert!(wide.ends_with("esc to stop"));
        assert_eq!(text::cells(&wide), 80);
        assert!(at(Duration::from_millis(3700), 80).contains("  ·  3.7s"));
        assert!(at(Duration::from_secs(64), 80).contains("1m 04s"));
        // The hint leaves whole before the sentence shortens.
        assert_eq!(at(Duration::ZERO, 32), "••••  Preparing  ·  0.0s");
        assert!(at(Duration::ZERO, 15).ends_with('…'));
    }

    #[test]
    fn phase_changes_reset_only_the_displayed_timer() {
        let mut activity = Activity::new();
        activity.started -= Duration::from_secs(10);
        activity.phase_started = activity.started;
        activity.set_label("Waiting");
        assert!(activity.phase_started > activity.started);
        let waiting_since = activity.phase_started;
        activity.set_label("Waiting");
        assert_eq!(activity.phase_started, waiting_since);
        assert!(
            activity
                .line(80, "")
                .plain()
                .contains("Waiting for the model")
        );
    }
}
