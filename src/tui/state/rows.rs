#[cfg(test)]
use super::State;
use super::{Item, Kind};
use crate::tui::{
    cards,
    line::Line,
    markdown, text,
    theme::{PROMPT, USER_BACKGROUND},
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
    // None: no visible predecessor. true: the preceding tool tree continues.
    pub previous: Option<bool>,
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
                let mut line = Line::new(&text::clean(text), PROMPT).at_source(0);
                line.background = Some(USER_BACKGROUND);
                let mut lines = vec![Line {
                    background: Some(USER_BACKGROUND),
                    ..Line::default()
                }];
                lines.extend(
                    line.wrap(columns.saturating_sub(4), true)
                        .into_iter()
                        .map(|line| line.indent("  ", PROMPT)),
                );
                lines.push(Line {
                    background: Some(USER_BACKGROUND),
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
                last,
                presentation,
                ..
            } => cards::render(
                cards::Tool {
                    name,
                    arguments,
                    summary,
                    result: result.as_ref(),
                    last: last.unwrap_or(false),
                    preview: *last != Some(false),
                    presentation,
                },
                columns,
            ),
        };
        let mut lines = lines;
        self.finish(item, &mut lines);
        lines
    }
    pub fn finish(&mut self, item: &Item, lines: &mut Vec<Line>) {
        if !lines.is_empty() {
            let continuing = matches!(item, Item::Tool { .. }) && self.previous == Some(true);
            if self.previous.is_some() && !continuing {
                lines.insert(0, Line::default());
            }
            self.previous = Some(matches!(
                item,
                Item::Tool {
                    last: Some(false),
                    ..
                }
            ));
        }
    }
}
