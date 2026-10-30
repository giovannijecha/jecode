use super::{
    line::Line,
    theme::{BASE, BODY, Surface},
    view::Frame,
};

// Paint the full alternate screen by absolute position. Each cursor move
// cancels delayed wrapping after a row, including the bottom-right cell.
#[derive(Default)]
pub struct Renderer {
    previous: Vec<Line>,
    size: (usize, usize),
    cursor: Option<(usize, usize)>,
    base: Surface,
}

impl Renderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_light_background(&mut self, light: bool) {
        let base = if light {
            Surface::rgb(12, 12, 12)
        } else {
            BASE
        };
        if self.base != base {
            self.base = base;
            self.previous.clear();
            self.size = (0, 0);
        }
    }

    pub fn paint(&mut self, frame: &Frame, width: usize, height: usize) -> String {
        let resized = self.size != (width, height);
        if !resized && self.previous == frame.live && self.cursor == frame.cursor {
            return String::new();
        }
        let columns = width.max(1);
        let mut output = String::from("\x1b[?25l");
        // Erasing fills with the current background only on terminals with
        // BCE, so rows are painted cell by cell; the erase just drops stale
        // glyphs a mis-measured row could leave behind.
        let erase = format!("\x1b[0m\x1b[{BODY};{}m\x1b[2K", self.base);
        if resized {
            output.push_str(&format!("\x1b[0m\x1b[{BODY};{}m\x1b[2J", self.base));
        }
        for (row, line) in frame.live.iter().enumerate().take(height) {
            if resized || self.previous.get(row) != Some(line) {
                output.push_str(&format!(
                    "\x1b[{};1H{erase}{}",
                    row + 1,
                    line.paint_on(columns, self.base)
                ));
            }
        }
        let stale = if resized {
            height
        } else {
            self.previous.len().min(height)
        };
        let blank = Line::default().paint_on(columns, self.base);
        for row in frame.live.len()..stale {
            output.push_str(&format!("\x1b[{};1H{erase}{blank}", row + 1));
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
