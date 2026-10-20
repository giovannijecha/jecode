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
        self.items
            .iter()
            .map(|item| rows.render(item, columns))
            .collect()
    }
}

#[derive(Default, Clone, Copy)]
pub(in crate::tui) struct Rows {
    pub previous: bool,
}

impl Rows {
    pub fn render(&mut self, item: &Item, columns: usize) -> Vec<Line> {
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
            Item::Tool {
                name,
                arguments,
                summary,
                result,
                change,
                presentation,
                ..
            } => cards::render(
                cards::Tool {
                    name,
                    arguments,
                    summary,
                    result: result.as_ref(),
                    change: change.as_deref(),
                    presentation,
                },
                columns,
            ),
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
