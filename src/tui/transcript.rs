use super::{
    line::Line,
    markdown,
    state::{
        Item, Kind, State,
        rows::{self, Rows},
    },
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

fn render(
    state: &State,
    block: usize,
    context: &mut Rows,
    columns: usize,
    keycap: Option<usize>,
) -> Vec<Line> {
    let Some(index) = block.checked_sub(1) else {
        *context = Rows::default();
        return header(columns);
    };
    context.render_at(&state.items, index, columns, keycap == Some(index))
}

// The card that names Ctrl+O: the last one with hidden lines in the latest
// finished turn, while that turn is collapsed.
type KeycapKey = (u64, u64, Option<(usize, usize)>, bool, bool, usize);

fn keycap_key(state: &State, columns: usize) -> KeycapKey {
    let details = &state.tool_details;
    (
        state.generation,
        state.tool_revision,
        details.turn.as_ref().map(|turn| (turn.start, turn.end)),
        details.active,
        details.expanded,
        columns,
    )
}

fn keycap(state: &State, columns: usize) -> Option<usize> {
    if state.tool_details.expanded || !state.tool_details_available() {
        return None;
    }
    state.tool_details.turn.clone()?.rev().find(|&index| {
        rows::tool_card(&state.items, index, columns, false).is_some_and(|card| card.hidden)
    })
}

pub(super) fn header(columns: usize) -> Vec<Line> {
    let mut title = Line::new(">_ ", ACCENT);
    title.push("Jecode", EMPHASIS.bold());
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
    partial: Option<Partial>,
    pending: Vec<Vec<Line>>,
    pending_start: usize,
    pending_revision: Option<u64>,
    pending_stream: Option<PendingStream>,
    keycap: Option<usize>,
    keycap_key: Option<KeycapKey>,
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
            self.partial = None;
            self.pending.clear();
            self.pending_revision = None;
            self.pending_stream = None;
            self.columns = columns;
        }
        if self.tools != state.tool_revision {
            // Several reads can finish in one event batch. Preserve the
            // settled prefix while redrawing from the earliest changed card.
            let block = state
                .tool_details
                .changed_since(state.tool_revision.wrapping_sub(self.tools))
                .map_or(1, |first| first + 1);
            if block < self.blocks.len() {
                self.context = self.contexts[block];
                self.blocks.truncate(block);
                self.contexts.truncate(block);
            }
            self.partial = None;
            self.pending_revision = None;
            self.pending_stream = None;
            self.tools = state.tool_revision;
        }
        self.refresh_keycap(state);
        let end = state.settled_len() + 1;
        let mut rows_left = 256usize;
        let mut blocks_left = max_blocks;
        while self.blocks.len() < end && blocks_left > 0 && rows_left > 0 {
            let item = source(state, self.blocks.len());
            let previous = self.context;
            if self.partial.is_none()
                && let Source::Item(item) = &item
                && item.is_visible()
                && let Item::Text {
                    kind: Kind::Assistant,
                    text,
                } = item
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
                if matches!(item, Source::Item(_)) {
                    self.context.finish(&mut lines);
                }
                lines
            } else {
                let lines = render(
                    state,
                    self.blocks.len(),
                    &mut self.context,
                    columns,
                    self.keycap,
                );
                rows_left = rows_left.saturating_sub(lines.len().max(1));
                lines
            };
            self.contexts.push(previous);
            self.blocks.push(lines);
            blocks_left -= 1;
        }
    }

    pub fn ready(&self, state: &State) -> bool {
        self.blocks.len() == state.settled_len() + 1
            && self.tools == state.tool_revision
            && self.keycap_key == Some(keycap_key(state, self.columns))
    }

    // Both the card that loses the keycap and the one that gains it are drawn again.
    fn refresh_keycap(&mut self, state: &State) {
        let key = keycap_key(state, self.columns);
        if self.keycap_key == Some(key) {
            return;
        }
        self.keycap_key = Some(key);
        let keycap = keycap(state, self.columns);
        if keycap == self.keycap {
            return;
        }
        let block = self
            .keycap
            .into_iter()
            .chain(keycap)
            .min()
            .map_or(0, |index| index + 1);
        self.keycap = keycap;
        if block < self.blocks.len() {
            self.context = self.contexts[block];
            self.blocks.truncate(block);
            self.contexts.truncate(block);
            self.partial = None;
        }
        self.pending_revision = None;
        self.pending_stream = None;
    }

    pub fn prepare_pending(&mut self, state: &State, max_rows: usize) {
        self.refresh_keycap(state);
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
                if let Item::Streaming { text } = item
                    && item.is_visible()
                {
                    if self
                        .pending_stream
                        .as_ref()
                        .is_none_or(|stream| stream.index != index)
                    {
                        self.pending_stream = Some(PendingStream {
                            index,
                            prefix: usize::from(context.previous),
                            markdown: markdown::Stream::new(text, self.columns),
                        });
                        let mut lines = Vec::new();
                        if context.previous {
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
                    context.finish(&mut marker);
                } else {
                    if self
                        .pending_stream
                        .as_ref()
                        .is_some_and(|stream| stream.index == index)
                    {
                        self.pending_stream = None;
                    }
                    let lines = context.render_at(
                        &state.items,
                        start + index,
                        self.columns,
                        self.keycap == Some(start + index),
                    );
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
                .any(|item| matches!(item, Item::Streaming { .. }) && item.is_visible())
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
