#[cfg(test)]
use super::State;
use super::{Item, Kind};
use crate::tui::{
    cards,
    line::Line,
    markdown, text,
    theme::{STRONG, USER_SURFACE},
};

#[cfg(test)]
impl State {
    pub fn rows(&self) -> Vec<Vec<Line>> {
        let columns = self.width.max(2);
        let mut rows = Rows::default();
        (0..self.items.len())
            .map(|index| rows.render_at(&self.items, index, columns, false))
            .collect()
    }
}

#[derive(Default, Clone, Copy)]
pub(in crate::tui) struct Rows {
    pub previous: bool,
}

impl Item {
    pub(in crate::tui) fn tool(&self) -> Option<cards::Tool<'_>> {
        match self {
            Item::Tool {
                name,
                arguments,
                summary,
                result,
                change,
                presentation,
                ..
            } => Some(cards::Tool {
                name,
                arguments,
                summary,
                result: result.as_ref(),
                change: change.as_deref(),
                presentation,
            }),
            _ => None,
        }
    }

    pub(in crate::tui) fn groupable(&self) -> bool {
        self.tool().is_some_and(|tool| cards::groupable(&tool))
    }
}

/// A tool's card. A run of reads shares one card, drawn at its first read;
/// the later reads draw nothing.
pub(in crate::tui) fn tool_card(
    items: &[Item],
    index: usize,
    columns: usize,
    keycap: bool,
) -> Option<cards::Card> {
    let tool = items[index].tool()?;
    if cards::groupable(&tool) {
        if index > 0 && items[index - 1].groupable() {
            return None;
        }
        let run: Vec<_> = items[index..]
            .iter()
            .map_while(|item| item.tool().filter(|tool| cards::groupable(tool)))
            .collect();
        if run.len() > 1 {
            return Some(cards::group(&run, columns, keycap));
        }
    }
    Some(cards::card(tool, columns, keycap))
}

impl Rows {
    #[cfg(test)]
    pub fn render(&mut self, item: &Item, columns: usize) -> Vec<Line> {
        self.render_at(std::slice::from_ref(item), 0, columns, false)
    }
    /// `items[index]`, with `keycap` naming Ctrl+O on its hidden lines.
    pub fn render_at(
        &mut self,
        items: &[Item],
        index: usize,
        columns: usize,
        keycap: bool,
    ) -> Vec<Line> {
        let item = &items[index];
        if !item.is_visible() {
            return Vec::new();
        }
        let lines = match item {
            Item::Streaming { text } => markdown::render(text, columns),
            Item::Text {
                kind: Kind::User,
                text,
            } => {
                let mut line = Line::new(&text::clean(text), STRONG).at_source(0);
                line.background = Some(USER_SURFACE);
                let mut lines = vec![Line {
                    background: Some(USER_SURFACE),
                    ..Line::default()
                }];
                lines.extend(
                    line.wrap(columns.saturating_sub(4), true)
                        .into_iter()
                        .map(|line| line.indent("  ", STRONG)),
                );
                lines.push(Line {
                    background: Some(USER_SURFACE),
                    ..Line::default()
                });
                lines
            }
            Item::Text {
                kind: Kind::Assistant,
                text,
            } => markdown::render(text, columns),
            Item::Local { .. } | Item::Text { .. } => Vec::new(),
            Item::Tool { .. } => {
                tool_card(items, index, columns, keycap).map_or_else(Vec::new, |card| card.rows)
            }
        };
        let mut lines = lines;
        self.finish(&mut lines);
        lines
    }
    pub fn finish(&mut self, lines: &mut Vec<Line>) {
        if !lines.is_empty() {
            if self.previous {
                lines.insert(0, Line::default());
            }
            self.previous = true;
        }
    }
}
