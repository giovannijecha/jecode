use super::{
    line::Line,
    render::Renderer,
    resize::Resize,
    state::{self, State},
    transcript::Layout,
    view,
    viewport::{Scroll, Viewport},
};
use std::{
    io::{self, Write},
    ops::Range,
    time::{Duration, Instant},
};

const LAYOUT_BLOCKS: usize = 64;

pub(super) struct Display {
    renderer: Renderer,
    resize: Resize,
    layout: Layout,
    viewport: Viewport,
    generation: Option<u64>,
    expanded_turn: Option<Range<usize>>,
    visible: Vec<Line>,
    working: bool,
    output_revision: Option<u64>,
    below: usize,
    fresh: bool,
    light_background: bool,
    /// The active console screen, opened on the first frame.
    #[cfg(windows)]
    console: Option<std::fs::File>,
}

impl Display {
    pub fn new(size: (usize, usize)) -> Self {
        Self {
            renderer: Renderer::new(),
            resize: Resize::new(size),
            layout: Layout::default(),
            viewport: Viewport::default(),
            generation: None,
            expanded_turn: None,
            visible: vec![],
            working: true,
            output_revision: None,
            below: 0,
            fresh: false,
            light_background: false,
            #[cfg(windows)]
            console: None,
        }
    }

    pub fn resized(&mut self, size: (usize, usize), now: Instant) {
        self.resize.observe(size, now);
    }
    pub fn reset(&mut self, size: (usize, usize)) {
        let light = self.light_background;
        *self = Self::new(size);
        self.set_light_background(light);
    }
    pub fn set_light_background(&mut self, light: bool) {
        self.light_background = light;
        self.renderer.set_light_background(light);
    }
    pub fn scroll(&mut self, request: Scroll) {
        self.viewport.scroll(request);
    }

    #[cfg(test)]
    pub fn back_to_bottom(&mut self) -> bool {
        if self.viewport.following() {
            return false;
        }
        self.viewport.scroll(Scroll::End);
        true
    }

    fn remap_collapsed_chapter(&mut self, state: &State) {
        let expanded = state
            .tool_details
            .expanded
            .then(|| state.tool_details.turn.clone())
            .flatten();
        if self.expanded_turn == expanded {
            return;
        }
        if let Some(previous) = self.expanded_turn.take() {
            self.viewport.remap_anchor(|block, origin| {
                let index = block.checked_sub(1)?;
                if !previous.contains(&index) {
                    return None;
                }
                let tool = state.items.get(index)?.tool()?;
                if !state::rows::in_chapter(&state.items, index) || tool.presentation.expanded {
                    return None;
                }
                let start = state::rows::chapter_start(&state.items, index);
                (index > start || origin.is_some_and(|origin| origin.line >= 2))
                    .then_some(start + 1)
            });
        }
        self.expanded_turn = expanded;
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
            self.write_frame(&format!("\x1b[?2026h{output}\x1b[?2026l"))
                .map_err(|error| format!("Could not draw terminal: {error}"))?;
        }
        Ok(())
    }

    /// Writes one frame with a single console call where possible. Standard
    /// output splits console writes into small chunks, and ConPTY paints a
    /// synchronized frame that arrives in several writes only at its next
    /// tick, about 16 ms later. The adapter selects UTF-8 console output.
    fn write_frame(&mut self, frame: &str) -> io::Result<()> {
        let mut stdout = io::stdout().lock();
        stdout.flush()?;
        #[cfg(windows)]
        {
            if self.console.is_none() {
                self.console = std::fs::OpenOptions::new().write(true).open("CONOUT$").ok();
            }
            if let Some(console) = &mut self.console {
                return console.write_all(frame.as_bytes());
            }
        }
        stdout.write_all(frame.as_bytes())?;
        stdout.flush()
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
            self.expanded_turn = None;
            self.visible.clear();
            self.generation = Some(state.generation);
            self.output_revision = None;
        }
        if self
            .output_revision
            .is_some_and(|previous| previous != state.output_revision)
        {
            self.viewport.new_output();
        }
        self.output_revision = Some(state.output_revision);
        let columns = state.width;
        let chrome = view::chrome(state, model, directory);
        let capacity = chrome.capacity(state.height);
        let mut reading = false;
        let dragging = self.resize.pending() && !self.resize.take_due(now);
        if !dragging {
            self.layout.prepare(state, columns, LAYOUT_BLOCKS);
            self.working = !self.layout.ready(state);
            if !self.working {
                self.remap_collapsed_chapter(state);
                self.layout.prepare_pending(state, 256);
                self.working = !self.layout.pending_ready(state);
                let slice =
                    self.viewport
                        .view(&self.layout.blocks, self.layout.pending_blocks(), capacity);
                self.visible = slice.lines;
                reading = slice.reading;
                (self.below, self.fresh) = (slice.below, slice.fresh);
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
        // Inside a collaborator in `/agents`, its conversation takes the column.
        if let Some(frame) = view::agents_inside(state) {
            return self.renderer.paint(&frame, state.width, state.height);
        }
        let mut frame = view::lower_with_chrome(lines, state.height, chrome);
        if frame.composer >= 2 {
            let gap = &mut frame.live[frame.composer - 1];
            if reading || !self.viewport.following() {
                *gap = view::reading_row(columns, self.below, self.fresh);
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

#[cfg(test)]
mod tests;
