//! Relative tail updates in the normal terminal buffer; never erase scrollback.
use super::{style::Row, text};
use std::fmt::Write;

#[derive(Default)]
pub struct Renderer {
    previous: Vec<Row>,
    size: (usize, usize),
}
impl Renderer {
    /// Replace only transient UI while retaining exactly the emitted transcript.
    /// Source received during drag-resize is presented after geometry settles.
    pub fn with_chrome(&self, chrome: Vec<Row>) -> Vec<Row> {
        let end = self
            .previous
            .iter()
            .position(|row| row.transient)
            .unwrap_or(self.previous.len());
        let mut frame = self.previous[..end].to_vec();
        frame.extend(chrome);
        frame
    }

    pub fn draw(&mut self, frame: Vec<Row>, size: (usize, usize), color: bool) -> String {
        if size.0 < 2 || size.1 < 2 {
            return String::new();
        }
        if frame == self.previous && size == self.size {
            return String::new();
        }
        let mut output = String::new();
        let visible = size.1.saturating_sub(1);
        let first = frame
            .iter()
            .zip(&self.previous)
            .position(|(a, b)| a != b)
            .unwrap_or(frame.len().min(self.previous.len()));
        let chrome = self
            .previous
            .iter()
            .position(|row| row.transient)
            .unwrap_or(self.previous.len());
        // Reflow belongs to the terminal, including rows already in scrollback.
        // Only mutable output and chrome are replaced, relative to their end.
        let mut start = first.min(chrome).min(frame.len());
        let height = |row: &Row| text::width(&row.text).div_ceil(size.0).max(1);
        let mut up: usize = self.previous[start..].iter().map(height).sum();
        // Rows outside the viewport cannot be restyled without replaying history.
        while up > visible && start < chrome && start < frame.len() {
            up -= height(&self.previous[start]);
            start += 1;
        }
        if up > 0 {
            let _ = write!(output, "\x1b[{}A", up.min(visible));
        }
        output.push_str("\r\x1b[J");
        for row in &frame[start..] {
            output.push_str(&row.paint(color));
            output.push_str("\r\n");
        }
        self.previous = frame;
        self.size = size;
        output
    }
}
