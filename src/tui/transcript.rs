use std::ops::Range;

use super::{
    line::Line,
    prose,
    state::{
        Item, Kind, State,
        rows::{self, Rows},
    },
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
        return super::view::opening::rows(state, columns);
    };
    context.render_at(&state.items, index, columns, keycap == Some(index))
}

// The latest WORK footer names expand; without a chapter, the last card with
// hidden content does. The first tool card names collapse for the whole turn.
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
    if !state.tool_details_available() {
        return None;
    }
    if state.tool_details.expanded {
        // Expanded, the turn's first card names the way back: `collapse`.
        return state
            .tool_details
            .turn
            .clone()?
            .find(|&index| state.items[index].is_visible() && state.items[index].tool().is_some());
    }
    let turn = state.tool_details.turn.clone()?;
    if let Some(index) = turn.clone().rev().find(|&index| {
        rows::in_chapter(&state.items, index) && rows::chapter_start(&state.items, index) == index
    }) {
        return Some(index);
    }
    turn.rev().find(|&index| {
        rows::tool_card(&state.items, index, columns, false).is_some_and(|card| card.hidden)
    })
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
    /// Built blocks to render again in place; later blocks stay cached
    /// unless the separator context after the range changes.
    stale: Option<Range<usize>>,
    /// Rewrap the stale range from its end, where the reader usually is.
    tail_first: bool,
    pending: Vec<Vec<Line>>,
    pending_start: usize,
    pending_revision: Option<u64>,
    pending_stream: Option<PendingStream>,
    /// The motion frame running cards were last drawn at.
    pending_frame: Option<usize>,
    keycap: Option<usize>,
    keycap_key: Option<KeycapKey>,
}

/// What one layout pass may still draw.
struct Budget {
    rows: usize,
    blocks: usize,
}

struct Partial {
    block: usize,
    markdown: prose::Rows,
    lines: Vec<Line>,
}

struct PendingStream {
    index: usize,
    prefix: usize,
    markdown: prose::Stream,
}

impl Layout {
    pub fn prepare(&mut self, state: &State, columns: usize, max_blocks: usize) {
        if self.generation != Some(state.generation) || self.blocks.is_empty() {
            self.blocks.clear();
            self.contexts.clear();
            self.context = Rows::default();
            self.generation = Some(state.generation);
            self.tools = state.tool_revision;
            self.partial = None;
            self.stale = None;
            self.pending.clear();
            self.pending_revision = None;
            self.pending_stream = None;
            self.tail_first = false;
            self.columns = columns;
        } else if columns != self.columns {
            // Built blocks stay on the page, shortened to the new width, while
            // they are wrapped again from the end of the conversation.
            self.columns = columns;
            self.partial = None;
            self.pending.clear();
            self.pending_revision = None;
            self.pending_stream = None;
            self.invalidate(1..self.blocks.len());
            self.tail_first = self.stale.is_some();
        }
        if self.tools != state.tool_revision {
            // Several reads can finish in one event batch. Preserve the
            // settled prefix and draw again only the changed cards.
            let items = state
                .tool_details
                .changed_since(state.tool_revision.wrapping_sub(self.tools))
                .unwrap_or(0..usize::MAX);
            // Prose and a stream outside the change keep their progress, so a
            // burst of tool events cannot restart a long rebuild indefinitely.
            let blocks = items.start + 1..items.end.saturating_add(1);
            if self
                .partial
                .as_ref()
                .is_some_and(|partial| blocks.contains(&partial.block))
            {
                self.partial = None;
            }
            self.forget_stream(&items);
            self.invalidate(blocks);
            self.pending_revision = None;
            self.tools = state.tool_revision;
        }
        // The header follows the model and effort, and the startup space
        // leaves with the first prompt.
        if let Some(header) = self.blocks.first_mut() {
            *header = super::view::opening::rows(state, columns);
        }
        self.refresh_keycap(state);
        let end = state.settled_len() + 1;
        let mut budget = Budget {
            rows: 256,
            blocks: max_blocks,
        };
        // New settled items come first, so they never wait for a long rebuild,
        // unless the single partial prose block belongs to that rebuild.
        if self
            .partial
            .as_ref()
            .is_none_or(|partial| partial.block >= self.blocks.len())
        {
            self.append(state, end, &mut budget);
            self.rebuild(state, &mut budget);
        } else {
            self.rebuild(state, &mut budget);
            self.append(state, end, &mut budget);
        }
    }

    /// Draws stale blocks again in place, from either end of the range.
    fn rebuild(&mut self, state: &State, budget: &mut Budget) {
        while budget.blocks > 0
            && budget.rows > 0
            && let Some(stale) = self.stale.clone()
        {
            let block = if self.tail_first {
                stale.end - 1
            } else {
                stale.start
            };
            let mut context = self.contexts[block];
            let Some(lines) = self.bounded(state, block, &mut context, &mut budget.rows) else {
                break;
            };
            budget.blocks -= 1;
            self.blocks[block] = lines;
            let next = block + 1;
            // A changed separator carries on to the next block.
            let carried = next < self.blocks.len() && context != self.contexts[next];
            if carried {
                self.contexts[next] = context;
            } else if next == self.blocks.len() {
                self.context = context;
            }
            self.stale = if self.tail_first {
                let end = if carried { next + 1 } else { block };
                (stale.start < end).then_some(stale.start..end)
            } else if next < stale.end || carried {
                Some(next..stale.end.max(next + 1))
            } else {
                None
            };
        }
        self.tail_first &= self.stale.is_some();
    }

    /// Builds the first blocks of newly settled items.
    fn append(&mut self, state: &State, end: usize, budget: &mut Budget) {
        while self.blocks.len() < end && budget.blocks > 0 && budget.rows > 0 {
            let previous = self.context;
            let mut context = previous;
            let Some(lines) =
                self.bounded(state, self.blocks.len(), &mut context, &mut budget.rows)
            else {
                break;
            };
            self.context = context;
            self.contexts.push(previous);
            self.blocks.push(lines);
            budget.blocks -= 1;
        }
    }

    /// New and invalidated prose share the same row budget. A partial rebuild
    /// leaves the old cached block visible until its replacement is complete.
    fn bounded(
        &mut self,
        state: &State,
        block: usize,
        context: &mut Rows,
        rows_left: &mut usize,
    ) -> Option<Vec<Line>> {
        if self
            .partial
            .as_ref()
            .is_some_and(|partial| partial.block != block)
        {
            self.partial = None;
        }
        if self.partial.is_none()
            && let Source::Item(item) = source(state, block)
            && item.is_visible()
            && let Item::Text {
                kind: Kind::Assistant,
                text,
            } = item
        {
            self.partial = Some(Partial {
                block,
                markdown: prose::Rows::new(text, self.columns),
                lines: vec![],
            });
        }
        if let Some(partial) = &mut self.partial {
            while *rows_left > 0 {
                let Some(lines) = partial.markdown.next() else {
                    let mut lines = self.partial.take().unwrap().lines;
                    context.finish(&mut lines);
                    return Some(lines);
                };
                *rows_left = rows_left.saturating_sub(lines.len().max(1));
                partial.lines.extend(lines);
            }
            None
        } else {
            let lines = render(state, block, context, self.columns, self.keycap);
            *rows_left = rows_left.saturating_sub(lines.len().max(1));
            Some(lines)
        }
    }

    pub fn ready(&self, state: &State) -> bool {
        self.blocks.len() == state.settled_len() + 1
            && self.stale.is_none()
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
        let last = self
            .keycap
            .into_iter()
            .chain(keycap)
            .max()
            .map_or(0, |index| index + 2);
        self.keycap = keycap;
        self.forget_stream(&(block.saturating_sub(1)..last.saturating_sub(1)));
        self.invalidate(block..last);
        self.pending_revision = None;
    }

    /// Marks built blocks to render again; blocks not built yet follow the
    /// source when they are.
    fn invalidate(&mut self, blocks: Range<usize>) {
        let blocks = blocks.start..blocks.end.min(self.blocks.len());
        if blocks.is_empty() {
            return;
        }
        self.stale = Some(match self.stale.take() {
            Some(stale) => stale.start.min(blocks.start)..stale.end.max(blocks.end),
            None => blocks,
        });
    }

    /// Drops the incremental stream when the changed items include it.
    fn forget_stream(&mut self, items: &Range<usize>) {
        if self
            .pending_stream
            .as_ref()
            .is_some_and(|stream| items.contains(&(self.pending_start + stream.index)))
        {
            self.pending_stream = None;
        }
    }

    /// Where the mutable tail begins: the first item without a built block.
    /// Items that settle while a rebuild holds them back join the tail.
    fn pending_from(&self, state: &State) -> usize {
        state.settled_len().min(self.blocks.len().saturating_sub(1))
    }

    /// Settled items still waiting for their first block.
    pub fn backlog(&self, state: &State) -> usize {
        (state.settled_len() + 1).saturating_sub(self.blocks.len())
    }

    /// The built blocks before the mutable tail.
    pub fn settled_blocks(&self) -> &[Vec<Line>] {
        &self.blocks[..(self.pending_start + 1).min(self.blocks.len())]
    }

    pub fn prepare_pending(&mut self, state: &State, max_rows: usize) {
        self.refresh_keycap(state);
        let start = self.pending_from(state);
        if start > self.pending_start && start - self.pending_start <= self.pending.len() {
            // Items now built as blocks leave the front of the tail; a stream
            // after them keeps its progress.
            let built = start - self.pending_start;
            self.pending.drain(..built);
            self.pending_stream = self.pending_stream.take().and_then(|mut stream| {
                stream.index = stream.index.checked_sub(built)?;
                Some(stream)
            });
            self.pending_start = start;
        }
        let changed = self.pending_revision != Some(state.revision) || self.pending_start != start;
        // Running cards animate and count time; draw them again once a frame.
        let frame = super::motion::frame();
        let running = self.pending_frame != Some(frame)
            && state.items[start..]
                .iter()
                .any(|item| matches!(item, Item::Tool { result: None, .. }));
        // The newest prose belongs to the live tail. Older settled answers
        // still waiting for a block are built by `prepare` with its row budget.
        let live_prose = state.items[start..].iter().rposition(|item| {
            item.is_visible()
                && matches!(
                    item,
                    Item::Streaming { .. }
                        | Item::Text {
                            kind: Kind::Assistant,
                            ..
                        }
                )
        });
        if changed || running {
            self.pending_frame = Some(frame);
            let reuse =
                self.pending_start == start && self.pending.len() == state.items.len() - start;
            if !reuse {
                self.pending.clear();
                self.pending_stream = None;
            }
            let mut context = self.context;
            for (index, item) in state.items[start..].iter().enumerate() {
                let prose = match item {
                    Item::Streaming { text }
                    | Item::Text {
                        kind: Kind::Assistant,
                        text,
                    } if item.is_visible() => Some(text.as_str()),
                    _ => None,
                };
                if let Some(text) = prose.filter(|_| live_prose == Some(index)) {
                    if self
                        .pending_stream
                        .as_ref()
                        .is_none_or(|stream| stream.index != index)
                    {
                        self.pending_stream = Some(PendingStream {
                            index,
                            prefix: usize::from(context.previous),
                            markdown: prose::Stream::new(text, self.columns),
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
                } else if prose.is_some() {
                    // Do not parse a settled answer in one unbounded frame.
                    // Its block replaces this temporary gap as `prepare` runs.
                    if index < self.pending.len() {
                        self.pending[index].clear();
                    } else {
                        self.pending.push(Vec::new());
                    }
                    context.previous = true;
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
            if live_prose.is_none() {
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
            && self.pending_start == self.pending_from(state)
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
mod expansion_tests;
#[cfg(test)]
mod markdown_tests;
#[cfg(test)]
mod rebuild_tests;
#[cfg(test)]
mod tests;
