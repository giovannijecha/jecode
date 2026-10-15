use super::{
    line::Line,
    markdown,
    state::{Item, Kind, State, rows::Rows},
    theme::{ACCENT, EMPHASIS},
};

enum Source<'a> {
    Header,
    Item(&'a Item),
}

fn source(state: &State, index: usize) -> Source<'_> {
    if index == 0 {
        Source::Header
    } else {
        Source::Item(&state.items[index - 1])
    }
}

fn render(source: Source<'_>, context: &mut Rows, columns: usize) -> Vec<Line> {
    match source {
        Source::Header => {
            *context = Rows::default();
            header(columns)
        }
        Source::Item(item) => context.render(item, columns),
    }
}

pub(super) fn header(columns: usize) -> Vec<Line> {
    let mut title = Line::new(">_ ", ACCENT);
    title.push("Jecode", EMPHASIS);
    let mut lines = title.wrap(columns, true);
    lines.push(Line::default());
    lines
}

// Settled blocks are laid out in bounded passes. Mutable blocks are rebuilt
// separately; neither cache changes the conversation or the saved session.
#[derive(Default)]
pub(super) struct Layout {
    pub blocks: Vec<Vec<Line>>,
    columns: usize,
    context: Rows,
    contexts: Vec<Rows>,
    generation: Option<u64>,
    tools: u64,
    selected_tool: Option<usize>,
    partial: Option<Partial>,
    pending: Vec<Vec<Line>>,
    pending_start: usize,
    pending_revision: Option<u64>,
    pending_stream: Option<PendingStream>,
}

struct Partial {
    markdown: markdown::Rows,
    lines: Vec<Line>,
}

struct PendingStream {
    index: usize,
    prefix: usize,
    markdown: markdown::Stream,
}

impl Layout {
    pub fn prepare(&mut self, state: &State, columns: usize, max_blocks: usize) {
        if columns != self.columns || self.generation != Some(state.generation) {
            self.blocks.clear();
            self.contexts.clear();
            self.context = Rows::default();
            self.generation = Some(state.generation);
            self.tools = state.tool_revision;
            self.selected_tool = state.tool_focus;
            self.partial = None;
            self.pending.clear();
            self.pending_revision = None;
            self.pending_stream = None;
            self.columns = columns;
        }
        if self.tools != state.tool_revision {
            for index in [
                self.selected_tool,
                state
                    .tool_focus
                    .filter(|&index| Some(index) != self.selected_tool),
            ]
            .into_iter()
            .flatten()
            {
                let block = index + 1;
                if block < self.blocks.len() {
                    let mut context = self.contexts[block];
                    self.blocks[block] = render(source(state, block), &mut context, columns);
                }
            }
            self.selected_tool = state.tool_focus;
            self.tools = state.tool_revision;
        }
        let end = state.settled_len() + 1;
        let mut rows_left = 256usize;
        let mut blocks_left = max_blocks;
        while self.blocks.len() < end && blocks_left > 0 && rows_left > 0 {
            let item = source(state, self.blocks.len());
            let previous = self.context;
            if self.partial.is_none()
                && let Source::Item(Item::Text {
                    kind: Kind::Assistant,
                    text,
                }) = &item
            {
                self.partial = Some(Partial {
                    markdown: markdown::Rows::new(text, columns),
                    lines: vec![],
                });
            }
            let lines = if let Some(partial) = &mut self.partial {
                let mut done = false;
                while rows_left > 0 {
                    let Some(lines) = partial.markdown.next() else {
                        done = true;
                        break;
                    };
                    rows_left = rows_left.saturating_sub(lines.len().max(1));
                    partial.lines.extend(lines);
                }
                if !done {
                    break;
                }
                let mut lines = self.partial.take().unwrap().lines;
                if let Source::Item(item) = item {
                    self.context.finish(item, &mut lines);
                }
                lines
            } else {
                let lines = render(item, &mut self.context, columns);
                rows_left = rows_left.saturating_sub(lines.len().max(1));
                lines
            };
            self.contexts.push(previous);
            self.blocks.push(lines);
            blocks_left -= 1;
        }
    }

    pub fn ready(&self, state: &State) -> bool {
        self.blocks.len() == state.settled_len() + 1 && self.tools == state.tool_revision
    }

    pub fn prepare_pending(&mut self, state: &State, max_rows: usize) {
        let start = state.settled_len();
        let changed = self.pending_revision != Some(state.revision) || self.pending_start != start;
        let running = state.items[start..]
            .iter()
            .any(|item| matches!(item, Item::Tool { result: None, .. }));
        if changed || running {
            let reuse =
                self.pending_start == start && self.pending.len() == state.items.len() - start;
            if !reuse {
                self.pending.clear();
                self.pending_stream = None;
            }
            let mut context = self.context;
            for (index, item) in state.items[start..].iter().enumerate() {
                if let Item::Streaming { text } = item {
                    if self
                        .pending_stream
                        .as_ref()
                        .is_none_or(|stream| stream.index != index)
                    {
                        self.pending_stream = Some(PendingStream {
                            index,
                            prefix: usize::from(context.previous.is_some()),
                            markdown: markdown::Stream::new(text, self.columns),
                        });
                        let mut lines = Vec::new();
                        if context.previous.is_some() {
                            lines.push(Line::default());
                        }
                        if index < self.pending.len() {
                            self.pending[index] = lines;
                        } else {
                            self.pending.push(lines);
                        }
                    } else if let Some(stream) = &mut self.pending_stream
                        && let Some(keep) = stream.markdown.update(text, self.columns)
                    {
                        self.pending[index].truncate(stream.prefix.max(keep));
                    }
                    // A stream is normally last, but a later pending item still
                    // needs the same separator context as full Rows::render.
                    let mut marker = vec![Line::default()];
                    context.finish(item, &mut marker);
                } else {
                    let lines = context.render(item, self.columns);
                    if index < self.pending.len() {
                        self.pending[index] = lines;
                    } else {
                        self.pending.push(lines);
                    }
                }
            }
            self.pending.truncate(state.items.len() - start);
            if !state.items[start..]
                .iter()
                .any(|item| matches!(item, Item::Streaming { .. }))
            {
                self.pending_stream = None;
            }
            self.pending_start = start;
            self.pending_revision = Some(state.revision);
        }
        if let Some(stream) = &mut self.pending_stream {
            stream
                .markdown
                .advance(&mut self.pending[stream.index], max_rows);
        }
    }

    pub fn pending_ready(&self, state: &State) -> bool {
        self.pending_revision == Some(state.revision)
            && self.pending_start == state.settled_len()
            && self
                .pending_stream
                .as_ref()
                .is_none_or(|stream| stream.markdown.ready())
    }

    pub fn pending_blocks(&self) -> &[Vec<Line>] {
        &self.pending
    }

    #[cfg(test)]
    pub fn pending(&mut self, state: &State) -> Vec<Line> {
        for _ in 0..1000 {
            self.prepare_pending(state, 256);
            if self.pending_ready(state) {
                break;
            }
        }
        self.pending_blocks().iter().flatten().cloned().collect()
    }
}

#[cfg(test)]
mod markdown_tests;
#[cfg(test)]
mod tests;
