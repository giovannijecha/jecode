#[cfg(test)]
use super::State;
use super::{Item, Kind};
use crate::tui::{
    cards,
    line::Line,
    prose, text,
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
        self.tool()
            .is_some_and(|tool| cards::chapter_groupable(&tool))
    }
}

fn same_chapter(left: &Item, right: &Item) -> bool {
    let (Some(left), Some(right)) = (left.tool(), right.tool()) else {
        return false;
    };
    cards::chapter_groupable(&left)
        && cards::chapter_groupable(&right)
        && left.presentation.turn_id == right.presentation.turn_id
        && cards::chapter_label(&left) == cards::chapter_label(&right)
}

/// Only runs of two or more routine calls become chapters.
pub(in crate::tui) fn in_chapter(items: &[Item], index: usize) -> bool {
    index > 0 && same_chapter(&items[index - 1], &items[index])
        || items
            .get(index + 1)
            .is_some_and(|next| same_chapter(&items[index], next))
}

/// Item positions stay stable in both views. Only the first member draws the
/// chapter heading; expanded calls retain their own viewport blocks.
pub(in crate::tui) fn chapter_start(items: &[Item], index: usize) -> usize {
    let Some(item) = items.get(index) else {
        return index;
    };
    if !item.groupable() {
        return index;
    }
    if let Some(start) = item.tool().and_then(|tool| tool.presentation.chapter_start) {
        return start;
    }
    let mut start = index;
    while start > 0 && same_chapter(&items[start - 1], item) {
        start -= 1;
        if let Some(cached) = items[start]
            .tool()
            .and_then(|tool| tool.presentation.chapter_start)
        {
            return cached;
        }
    }
    start
}

pub(in crate::tui) fn tool_card(
    items: &[Item],
    index: usize,
    columns: usize,
    keycap: bool,
) -> Option<cards::Card> {
    let tool = items[index].tool()?;
    if !in_chapter(items, index) {
        return Some(cards::card(tool, columns, keycap));
    }
    let first = index == 0 || !same_chapter(&items[index - 1], &items[index]);
    let mut chapter = if first {
        let calls: Vec<_> = items[index..]
            .iter()
            .take_while(|item| same_chapter(&items[index], item))
            .filter_map(Item::tool)
            .collect();
        cards::chapter_header(&calls, columns, keycap)
    } else {
        cards::Card {
            rows: Vec::new(),
            hidden: false,
        }
    };
    let mut detail = if tool.presentation.expanded {
        let rows = cards::card(tool, columns, false).rows;
        if first {
            chapter.rows.push(Line::default());
        }
        rows
    } else {
        cards::chapter_exception(&tool, columns)
    };
    if first {
        for line in &mut detail {
            if let Some(origin) = &mut line.origin {
                origin.line = origin.line.saturating_add(2);
            }
        }
    }
    chapter.rows.extend(detail);
    let last = index + 1 == items.len() || !same_chapter(&items[index], &items[index + 1]);
    if last {
        let start = chapter_start(items, index);
        let calls: Vec<_> = items[start..=index].iter().filter_map(Item::tool).collect();
        let activity = cards::chapter_activity(&calls, columns);
        let has_activity = activity.is_some();
        if !calls.iter().any(|tool| tool.presentation.expanded) {
            // Preserve the former singleton's content height with result
            // facts. A keycap is presentation, not content, and cannot set
            // the chapter's floor.
            let singleton = cards::card(calls[0], columns, false).rows.len();
            let chapter_height = cards::chapter_header(&calls, columns, false).rows.len()
                + calls
                    .iter()
                    .map(|tool| cards::chapter_exception(tool, columns).len())
                    .sum::<usize>()
                + usize::from(has_activity);
            chapter.rows.extend(
                cards::chapter_receipts(&calls[0], columns)
                    .into_iter()
                    .take(singleton.saturating_sub(chapter_height)),
            );
        }
        if let Some(activity) = activity {
            chapter.rows.push(activity);
        }
    }
    (!chapter.rows.is_empty()).then_some(chapter)
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
            Item::Streaming { text } => prose::render(text, columns),
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
            } => prose::render(text, columns),
            Item::Local { .. } | Item::Text { .. } => Vec::new(),
            Item::Tool { .. } => {
                tool_card(items, index, columns, keycap).map_or_else(Vec::new, |card| card.rows)
            }
        };
        let mut lines = lines;
        let joined = index > 0
            && same_chapter(&items[index - 1], item)
            && item.tool().is_some_and(|tool| !tool.presentation.expanded);
        if joined {
            self.previous |= !lines.is_empty();
        } else {
            self.finish(&mut lines);
        }
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
