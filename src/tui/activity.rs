use super::{
    line::Line,
    text,
    theme::{EMPHASIS, TOOL_DIM},
};
use std::time::{Duration, Instant};

pub struct Activity {
    pub started: Instant,
    pub label: &'static str,
    pub tools: usize,
    pub stopping: bool,
}
impl Activity {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            label: "Waiting",
            tools: 0,
            stopping: false,
        }
    }
    pub fn line(&self, columns: usize, hint: &str) -> Line {
        self.at(self.started.elapsed(), columns, hint)
    }
    fn at(&self, elapsed: Duration, columns: usize, hint: &str) -> Line {
        let mut line = indicator((elapsed.as_millis() / 120) as usize);
        line.push("  ", TOOL_DIM);
        line.push(
            if self.label == "Waiting" {
                "Waiting for the model"
            } else {
                self.label
            },
            EMPHASIS,
        );
        line.push(&format!("  ·  {}", duration(elapsed)), TOOL_DIM);
        let cells = text::cells(&line.plain());
        if columns >= cells + hint.len() + 3 {
            line.push(&" ".repeat(columns - cells - hint.len()), TOOL_DIM);
            line.push(hint, TOOL_DIM);
        }
        line.shortened(columns)
    }
}

pub(super) fn indicator(frame: usize) -> Line {
    // A constant glyph and weight keep all four dots on the same baseline.
    const SHADES: [&str; 4] = [
        "38;2;78;97;113",
        "38;2;105;133;159",
        "38;2;137;169;196",
        "38;2;181;213;241",
    ];
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
        line.push("•", SHADES[shade]);
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
    fn animation_has_fixed_positions_and_duration_spans_the_whole_turn() {
        let activity = Activity::new();
        assert!(
            activity
                .at(Duration::from_millis(3200), 80, "Esc stops")
                .plain()
                .starts_with("••••  Waiting for the model  ·  3.2s")
        );
        assert!(
            activity
                .at(Duration::from_millis(3700), 80, "Esc stops")
                .plain()
                .starts_with("••••  Waiting for the model  ·  3.7s")
        );
        assert!(
            activity
                .at(Duration::from_secs(64), 80, "Esc stops")
                .plain()
                .contains("1m 04s")
        );
        assert!(
            activity
                .at(Duration::ZERO, 80, "Esc stops")
                .plain()
                .ends_with("Esc stops")
        );
        assert!(
            !activity
                .at(Duration::ZERO, 15, "Esc stops")
                .plain()
                .contains("Esc stops")
        );
    }
}
