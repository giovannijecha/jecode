use super::{
    line::{Line, Span},
    render::Renderer,
    resize::Resize,
    state::State,
    transcript::Layout,
    view,
    viewport::{Scroll, Viewport},
};
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

const LAYOUT_BLOCKS: usize = 64;

pub(super) struct Display {
    renderer: Renderer,
    resize: Resize,
    layout: Layout,
    viewport: Viewport,
    generation: Option<u64>,
    pending: Vec<Vec<Line>>,
    pending_key: Option<(u64, usize, usize)>,
    visible: Vec<Line>,
    working: bool,
}

impl Display {
    pub fn new(size: (usize, usize)) -> Self {
        Self {
            renderer: Renderer::new(),
            resize: Resize::new(size),
            layout: Layout::default(),
            viewport: Viewport::default(),
            generation: None,
            pending: vec![],
            pending_key: None,
            visible: vec![],
            working: true,
        }
    }

    pub fn resized(&mut self, size: (usize, usize), now: Instant) {
        self.resize.observe(size, now);
    }
    pub fn reset(&mut self, size: (usize, usize)) {
        *self = Self::new(size);
    }
    pub fn scroll(&mut self, request: Scroll) {
        self.viewport.scroll(request);
    }

    pub fn back_to_bottom(&mut self) -> bool {
        if self.viewport.following() {
            return false;
        }
        self.viewport.scroll(Scroll::End);
        true
    }

    pub fn needs_draw(&self, state: &State, now: Instant) -> bool {
        state.width >= 2
            && state.height >= 2
            && (self.resize.due(now) || self.working && !self.resize.pending())
    }

    pub fn wait(&self, state: &State, now: Instant) -> Duration {
        if state.width < 2 || state.height < 2 {
            Duration::from_millis(30)
        } else if self.working && !self.resize.pending() {
            Duration::ZERO
        } else {
            self.resize.wait(now)
        }
    }

    pub fn draw(&mut self, state: &State, model: &str, directory: &str) -> Result<(), String> {
        let output = self.update(state, model, directory, Instant::now());
        if !output.is_empty() {
            write!(io::stdout(), "\x1b[?2026h{output}\x1b[?2026l")
                .and_then(|_| io::stdout().flush())
                .map_err(|error| format!("Could not draw terminal: {error}"))?;
        }
        Ok(())
    }

    pub(super) fn update(
        &mut self,
        state: &State,
        model: &str,
        directory: &str,
        now: Instant,
    ) -> String {
        if state.width < 2 || state.height < 2 {
            return String::new();
        }
        if self.generation != Some(state.generation) {
            self.viewport = Viewport::default();
            self.visible.clear();
            self.pending_key = None;
            self.generation = Some(state.generation);
        }
        let columns = state.width;
        let capacity = view::capacity(state, model, directory);
        let mut reading = false;
        let dragging = self.resize.pending() && !self.resize.take_due(now);
        if !dragging {
            self.layout.prepare(state, columns, LAYOUT_BLOCKS);
            self.working = !self.layout.ready(state);
            if !self.working {
                let key = (state.revision, columns, self.layout.blocks.len());
                let running = state.items[state.settled_len()..]
                    .iter()
                    .any(|item| matches!(item, super::state::Item::Tool { result: None, .. }));
                if self.pending_key != Some(key) || running {
                    self.pending = self.layout.pending_blocks(state);
                    self.pending_key = Some(key);
                }
                let slice = self
                    .viewport
                    .view(&self.layout.blocks, &self.pending, capacity);
                self.visible = slice.lines;
                reading = slice.reading;
            }
        }
        // Keep the current reading page during reflow. Layout runs between input
        // polls, and resize never clears the caller's native scrollback.
        let mut lines: Vec<_> = self
            .visible
            .iter()
            .map(|line| line.shortened(columns))
            .collect();
        if !self.viewport.following() {
            lines.truncate(capacity);
        }
        let mut frame = view::lower(
            state,
            model,
            directory,
            lines,
            true,
            state.height,
            !self.viewport.following(),
        );
        if frame.composer >= 2 {
            let gap = &mut frame.live[frame.composer - 1];
            if reading || !self.viewport.following() {
                *gap = back_to_bottom_line(columns, back_to_bottom_uses_alt_end(state));
            } else if dragging || self.working {
                *gap = Line::new(
                    &super::text::ellipsize("Reflowing conversation…", columns),
                    super::theme::MUTED,
                );
            }
        }
        self.renderer.paint(&frame, state.width, state.height)
    }
}

fn back_to_bottom_uses_alt_end(state: &State) -> bool {
    state.queue.is_editing()
        || state.history.is_browsing()
        || state.selector.is_some()
        || state.suggestions.panel
        || state.information.is_some()
        || state.tool_focus.is_some()
}

fn back_to_bottom_line(columns: usize, alt_end: bool) -> Line {
    const ESC_LABELS: [&str; 4] = [
        " ↓ Back to bottom · esc ",
        " ↓ Bottom · esc ",
        " ↓ esc ",
        "↓",
    ];
    const ALT_END_LABELS: [&str; 4] = [
        " ↓ Back to bottom · Alt+End ",
        " ↓ Bottom · Alt+End ",
        " ↓ Alt+End ",
        "↓",
    ];
    let labels = if alt_end { ALT_END_LABELS } else { ESC_LABELS };
    let label = labels
        .into_iter()
        .find(|label| super::text::cells(label) <= columns)
        .unwrap_or("");
    let margin = (columns.saturating_sub(super::text::cells(label))) / 2;
    let mut line = Line::new(&" ".repeat(margin), super::theme::BODY);
    if !label.is_empty() {
        line.spans.push(Span {
            text: label.into(),
            style: super::theme::ACCENT,
            background: Some(super::theme::TOOL_SELECTED),
        });
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::state::Kind;
    use crate::tui::theme::{ACCENT, BODY, TOOL_SELECTED};

    #[test]
    fn back_to_bottom_badge_is_centered_and_uses_the_blue_palette() {
        let line = back_to_bottom_line(40, false);
        assert_eq!(
            line.plain(),
            format!("{} ↓ Back to bottom · esc ", " ".repeat(8))
        );
        assert_eq!(line.spans[0].style, BODY);
        assert_eq!(line.spans[0].background, None);
        assert_eq!(line.spans[1].style, ACCENT);
        assert_eq!(line.spans[1].background, Some(TOOL_SELECTED));
        assert!(line.paint(40).contains(&format!("\x1b[{TOOL_SELECTED}m")));
    }

    #[test]
    fn back_to_bottom_badge_fits_small_widths() {
        for alt_end in [false, true] {
            for width in 0..30 {
                let line = back_to_bottom_line(width, alt_end);
                assert!(super::super::text::cells(&line.plain()) <= width);
            }
        }
        assert_eq!(back_to_bottom_line(1, false).plain(), "↓");
        assert_eq!(back_to_bottom_line(7, false).plain(), " ↓ esc ");
        assert_eq!(back_to_bottom_line(11, true).plain(), " ↓ Alt+End ");
    }

    #[test]
    fn contextual_escape_actions_use_alt_end_in_the_badge() {
        let mut state = State::default();
        assert!(!back_to_bottom_uses_alt_end(&state));
        state.history.record("saved prompt");
        state.history.navigate(&mut state.editor, true);
        assert!(back_to_bottom_uses_alt_end(&state));
        state.history.cancel(&mut state.editor);
        state.queue.push("queued draft").unwrap();
        state.queue.begin_edit(0, &mut state.editor).unwrap();
        assert!(back_to_bottom_uses_alt_end(&state));
        state.queue.cancel_edit(&mut state.editor);
        state.suggestions.panel = true;
        assert!(back_to_bottom_uses_alt_end(&state));
        state.suggestions.panel = false;
        state.tool_focus = Some(0);
        assert!(back_to_bottom_uses_alt_end(&state));
        assert!(
            back_to_bottom_line(40, true)
                .plain()
                .contains("Back to bottom · Alt+End")
        );
    }

    #[test]
    fn back_to_bottom_only_requests_the_end_while_reading() {
        let mut display = Display::new((40, 20));
        let blocks: Vec<_> = (0..20)
            .map(|index| vec![Line::new(&index.to_string(), BODY)])
            .collect();
        assert!(!display.back_to_bottom());
        display.viewport.view(&blocks, &[], 5);
        display.scroll(Scroll::Page(true));
        assert!(display.viewport.view(&blocks, &[], 5).reading);
        assert!(display.back_to_bottom());
        assert!(!display.viewport.view(&blocks, &[], 5).reading);
        assert!(!display.back_to_bottom());
    }

    #[test]
    fn badge_appears_above_the_composer_only_while_reading() {
        let mut state = State::default();
        state.message(
            Kind::Assistant,
            &(0..60)
                .map(|index| format!("Row {index:02}\n"))
                .collect::<String>(),
        );
        let mut display = Display::new((state.width, state.height));
        let now = Instant::now();
        let initial = display.update(&state, "fixture/model", "fixture directory", now);
        assert!(!initial.contains("Back to bottom"));
        display.scroll(Scroll::Page(true));
        let reading = display.update(&state, "fixture/model", "fixture directory", now);
        assert!(reading.contains("Back to bottom · esc"));
        state.history.record("saved prompt");
        state.history.navigate(&mut state.editor, true);
        let contextual = display.update(&state, "fixture/model", "fixture directory", now);
        assert!(contextual.contains("Back to bottom · Alt+End"));
        assert!(display.back_to_bottom());
        let following = display.update(&state, "fixture/model", "fixture directory", now);
        assert!(!following.contains("Back to bottom"));
    }

    #[test]
    fn short_conversations_do_not_show_a_badge_after_scroll_start() {
        let mut state = State::default();
        state.message(Kind::Assistant, "A short answer.");
        let mut display = Display::new((state.width, state.height));
        let now = Instant::now();
        display.update(&state, "fixture/model", "fixture directory", now);
        display.scroll(Scroll::Start);
        let output = display.update(&state, "fixture/model", "fixture directory", now);
        assert!(!output.contains("Back to bottom"));
        assert!(!display.back_to_bottom());
    }
}
