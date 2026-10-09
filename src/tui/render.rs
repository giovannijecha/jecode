use super::{line::Line, view::Frame};

// Paint the full alternate screen by absolute position. Each cursor move
// cancels delayed wrapping after a row, including the bottom-right cell.
#[derive(Default)]
pub struct Renderer {
    previous: Vec<Line>,
    size: (usize, usize),
    cursor: Option<(usize, usize)>,
}

impl Renderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn paint(&mut self, frame: &Frame, width: usize, height: usize) -> String {
        let resized = self.size != (width, height);
        if !resized && self.previous == frame.live && self.cursor == frame.cursor {
            return String::new();
        }
        let columns = width.max(1);
        let mut output = String::from("\x1b[?25l");
        if resized {
            output.push_str("\x1b[0m\x1b[2J");
        }
        for (row, line) in frame.live.iter().enumerate().take(height) {
            if resized || self.previous.get(row) != Some(line) {
                output.push_str(&format!(
                    "\x1b[{};1H\x1b[0m\x1b[2K{}",
                    row + 1,
                    line.paint(columns)
                ));
            }
        }
        for row in frame.live.len()..self.previous.len().min(height) {
            output.push_str(&format!("\x1b[{};1H\x1b[0m\x1b[2K", row + 1));
        }
        let (row, column) = frame.cursor.unwrap_or((frame.composer, 0));
        output.push_str(&format!(
            "\x1b[{};{}H",
            row.min(height.saturating_sub(1)) + 1,
            column.min(columns - 1) + 1
        ));
        self.previous.clone_from(&frame.live);
        self.size = (width, height);
        self.cursor = frame.cursor;
        output
    }

    #[cfg(test)]
    fn update(&mut self, frame: Frame, width: usize, height: usize) -> String {
        self.paint(&frame, width, height)
    }
}

#[cfg(test)]
mod tests;
