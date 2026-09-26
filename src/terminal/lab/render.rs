//! Relative tail updates in the normal terminal buffer. Scrollback is only
//! erased when a width change or an invalidated history forces a full replay.
//! Each paint is one synchronized update (DEC 2026), so terminals that
//! support it swap the frame atomically and others ignore the markers.
use super::caps::ColorDepth;
use super::style::Row;
use super::text;
use std::fmt::Write;

const BEGIN: &str = "\x1b[?2026h\x1b[?7l";
const END: &str = "\x1b[?7h\x1b[?2026l";

#[derive(Default)]
pub struct Renderer {
    previous: Vec<Row>,
    size: (usize, usize),
    /// The screen holds only a preview's tail; the next draw replays all.
    partial: bool,
}

impl Renderer {
    /// Bytes that turn the previous frame into `frame`; "" when nothing changed.
    pub fn draw(&mut self, frame: Vec<Row>, size: (usize, usize), depth: ColorDepth) -> String {
        if size.0 < 2 || size.1 < 2 || frame == self.previous && size == self.size && !self.partial
        {
            return String::new();
        }
        if self.partial || !self.previous.is_empty() && size.0 != self.size.0 {
            return self.replay(frame, 0, size, depth);
        }
        let first = frame
            .iter()
            .zip(&self.previous)
            .position(|(next, previous)| next != previous)
            .unwrap_or(frame.len().min(self.previous.len()));
        let chrome = self.chrome_start();
        let visible = size.1 - 1;
        let height = |row: &Row| text::width(&row.text).div_ceil(size.0).max(1);
        // Reflow belongs to the terminal, including rows already in scrollback.
        // Only mutable output and chrome are replaced, relative to their end.
        let mut start = first.min(chrome).min(frame.len());
        let mut up: usize = self.previous[start..].iter().map(height).sum();
        // Rows outside the viewport cannot be restyled without replaying history.
        while up > visible && start < chrome && start < frame.len() {
            up -= height(&self.previous[start]);
            start += 1;
        }
        let mut output = String::from(BEGIN);
        if up > 0 {
            let _ = write!(output, "\x1b[{}A", up.min(visible));
        }
        output.push_str("\r\x1b[J");
        for row in &frame[start..] {
            output.push_str(&row.paint(depth));
            output.push_str("\r\n");
        }
        output.push_str(END);
        self.previous = frame;
        self.size = size;
        output
    }

    /// The emitted history no longer matches (tool detail expanded or
    /// folded): the next `draw` replays the whole frame.
    pub fn invalidate(&mut self) {
        self.partial = true;
    }

    /// Mid-resize: the frame laid out for the current geometry, but only the
    /// rows that fit the window, so each step of a drag costs one screen.
    /// The next `draw` replays the whole frame, scrollback included.
    pub fn preview(&mut self, frame: Vec<Row>, size: (usize, usize), depth: ColorDepth) -> String {
        if size.0 < 2 || size.1 < 2 || self.partial && frame == self.previous && size == self.size {
            return String::new();
        }
        let from = frame.len().saturating_sub(size.1 - 1);
        let output = self.replay(frame, from, size, depth);
        self.partial = true;
        output
    }

    /// A width change: the terminal has rewrapped every emitted row on its
    /// own terms (clipped paths, right-aligned times and full-width rules all
    /// break), and how depends on the terminal, so nothing on screen can be
    /// trusted. Clear screen and scrollback, then emit the frame from row
    /// `from`, laid out for the new width, in one synchronized update.
    fn replay(
        &mut self,
        frame: Vec<Row>,
        from: usize,
        size: (usize, usize),
        depth: ColorDepth,
    ) -> String {
        let mut output = String::from(BEGIN);
        output.push_str("\x1b[H\x1b[2J\x1b[3J");
        for row in &frame[from..] {
            output.push_str(&row.paint(depth));
            output.push_str("\r\n");
        }
        output.push_str(END);
        self.previous = frame;
        self.size = size;
        self.partial = false;
        output
    }

    fn chrome_start(&self) -> usize {
        self.previous
            .iter()
            .position(|row| row.transient)
            .unwrap_or(self.previous.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::lab::style::Tone;

    fn rows(texts: &[&str]) -> Vec<Row> {
        texts
            .iter()
            .map(|text| Row::new(*text, Tone::Text))
            .collect()
    }

    #[test]
    fn first_paint_is_one_synchronized_update() {
        let mut renderer = Renderer::default();
        let out = renderer.draw(rows(&["a", "b"]), (10, 5), ColorDepth::None);
        assert_eq!(out, format!("{BEGIN}\r\x1b[Ja\r\nb\r\n{END}"));
    }

    #[test]
    fn unchanged_frame_writes_nothing() {
        let mut renderer = Renderer::default();
        renderer.draw(rows(&["a", "b"]), (10, 5), ColorDepth::None);
        assert_eq!(
            renderer.draw(rows(&["a", "b"]), (10, 5), ColorDepth::None),
            ""
        );
    }

    #[test]
    fn only_the_changed_tail_is_rewritten() {
        let mut renderer = Renderer::default();
        renderer.draw(rows(&["a", "b", "c"]), (10, 5), ColorDepth::None);
        let out = renderer.draw(rows(&["a", "b", "C"]), (10, 5), ColorDepth::None);
        assert_eq!(out, format!("{BEGIN}\x1b[1A\r\x1b[JC\r\n{END}"));
    }

    #[test]
    fn wide_rows_count_their_wrapped_height() {
        let mut renderer = Renderer::default();
        renderer.draw(rows(&["日本語テキスト", "x"]), (10, 8), ColorDepth::None);
        let out = renderer.draw(rows(&["y", "x"]), (10, 8), ColorDepth::None);
        assert!(out.contains("\x1b[3A"), "{out:?}");
    }

    #[test]
    fn a_width_change_replays_the_frame_on_a_clean_screen() {
        let mut renderer = Renderer::default();
        renderer.draw(rows(&["a", "b"]), (10, 5), ColorDepth::None);
        let out = renderer.draw(rows(&["a", "b"]), (8, 5), ColorDepth::None);
        assert_eq!(out, format!("{BEGIN}\x1b[H\x1b[2J\x1b[3Ja\r\nb\r\n{END}"));
    }

    #[test]
    fn a_preview_paints_one_screen_then_draw_replays_everything() {
        let mut renderer = Renderer::default();
        let frame = rows(&["a", "b", "c", "d"]);
        renderer.draw(frame.clone(), (10, 5), ColorDepth::None);
        let preview = renderer.preview(frame.clone(), (8, 3), ColorDepth::None);
        assert_eq!(
            preview,
            format!("{BEGIN}\x1b[H\x1b[2J\x1b[3Jc\r\nd\r\n{END}")
        );
        assert_eq!(
            renderer.preview(frame.clone(), (8, 3), ColorDepth::None),
            ""
        );
        let settled = renderer.draw(frame, (8, 3), ColorDepth::None);
        assert!(
            settled.ends_with(&format!("a\r\nb\r\nc\r\nd\r\n{END}")),
            "{settled:?}"
        );
    }

    #[test]
    fn an_invalidated_history_replays_even_an_unchanged_frame() {
        let mut renderer = Renderer::default();
        renderer.draw(rows(&["a", "b"]), (10, 5), ColorDepth::None);
        renderer.invalidate();
        let out = renderer.draw(rows(&["a", "b"]), (10, 5), ColorDepth::None);
        assert_eq!(out, format!("{BEGIN}\x1b[H\x1b[2J\x1b[3Ja\r\nb\r\n{END}"));
    }

    #[test]
    fn a_height_change_only_repaints_the_tail() {
        let mut renderer = Renderer::default();
        renderer.draw(rows(&["a", "b"]), (10, 5), ColorDepth::None);
        let out = renderer.draw(rows(&["a", "b"]), (10, 9), ColorDepth::None);
        assert!(!out.contains("\x1b[2J"), "{out:?}");
    }

    #[test]
    fn chrome_repaints_from_its_first_row() {
        let frame = |last: &str| {
            let mut frame = rows(&["t"]);
            let mut status = Row::new("status", Tone::Text);
            status.transient = true;
            frame.push(status);
            frame.push(Row::new(last, Tone::Text));
            frame
        };
        let mut renderer = Renderer::default();
        renderer.draw(frame("a"), (10, 5), ColorDepth::None);
        let out = renderer.draw(frame("b"), (10, 5), ColorDepth::None);
        assert_eq!(out, format!("{BEGIN}\x1b[2A\r\x1b[Jstatus\r\nb\r\n{END}"));
    }
}
