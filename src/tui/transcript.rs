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
    generation: Option<u64>,
    tools: u64,
    partial: Option<Partial>,
}

struct Partial {
    markdown: markdown::Rows,
    lines: Vec<Line>,
}

impl Layout {
    pub fn prepare(&mut self, state: &State, columns: usize, max_blocks: usize) {
        if columns != self.columns
            || self.generation != Some(state.generation)
            || self.tools != state.tool_revision
        {
            self.blocks.clear();
            self.context = Rows::default();
            self.generation = Some(state.generation);
            self.tools = state.tool_revision;
            self.partial = None;
            self.columns = columns;
        }
        let end = state.settled_len() + 1;
        let mut rows_left = 256usize;
        let mut blocks_left = max_blocks;
        while self.blocks.len() < end && blocks_left > 0 && rows_left > 0 {
            let item = source(state, self.blocks.len());
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
            self.blocks.push(lines);
            blocks_left -= 1;
        }
    }

    pub fn ready(&self, state: &State) -> bool {
        self.blocks.len() == state.settled_len() + 1 && self.tools == state.tool_revision
    }

    pub fn pending_blocks(&self, state: &State) -> Vec<Vec<Line>> {
        let mut context = self.context;
        state.items[state.settled_len()..]
            .iter()
            .map(|item| context.render(item, self.columns))
            .collect()
    }

    #[cfg(test)]
    pub fn pending(&self, state: &State) -> Vec<Line> {
        self.pending_blocks(state).into_iter().flatten().collect()
    }
}

#[cfg(test)]
mod markdown_tests;
#[cfg(test)]
mod tests;
