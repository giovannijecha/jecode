#[cfg(test)]
use super::State;
use super::{Item, Kind};
use crate::tui::{
    cards,
    line::Line,
    prose, text,
    theme::{ACCENT, BAD, BADGE_IDLE, BODY, LIT, STRONG, USER_SURFACE},
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
        // Expanded, the title's tree closes on its facts; each full card
        // below carries its own.
        cards::hang(&mut chapter.rows, true);
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
        let activities = cards::chapter_activities(&calls, columns);
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
                + activities.len();
            chapter.rows.extend(
                cards::chapter_receipts(&calls[0], columns)
                    .into_iter()
                    .take(singleton.saturating_sub(chapter_height)),
            );
            // Expanded chapters skip it: every call shows as its full card.
            chapter.rows.extend(activities);
        }
    }
    // Collapsed, one tree runs from the WORK title through the members and
    // closes on the chapter's last row.
    if !tool.presentation.expanded {
        let titled = usize::from(first).min(chapter.rows.len());
        cards::hang(&mut chapter.rows[titled..], last);
    }
    (!chapter.rows.is_empty()).then_some(chapter)
}

/// The sent request: `› ` on the full-row user surface between two padding
/// rows, the text hanging at column 2 and attachment labels drawn as chips.
fn request(value: &str, columns: usize) -> Vec<Line> {
    let mut line = chips(&text::clean(value)).at_source(0);
    line.background = Some(USER_SURFACE);
    let pad = || Line {
        background: Some(USER_SURFACE),
        ..Line::default()
    };
    let mut lines = vec![pad()];
    lines.extend(
        line.wrap(columns.saturating_sub(4), true)
            .into_iter()
            .enumerate()
            .map(|(index, line)| line.indent(if index == 0 { "› " } else { "  " }, ACCENT.bold())),
    );
    lines.push(pad());
    lines
}

/// `[1# Image]` and `[2# File: wake.log]` become ` 1 Image ` and ` 2 wake.log `
/// in LIT on the quiet badge surface; inner spaces never break.
fn chips(value: &str) -> Line {
    let mut line = Line::default();
    let mut rest = value;
    while let Some(start) = rest.find('[') {
        let tail = &rest[start + 1..];
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        let label = tail[digits..]
            .strip_prefix("# ")
            .and_then(|label| label.find(']').map(|end| &label[..end]))
            .filter(|label| digits > 0 && !label.is_empty());
        let Some(label) = label else {
            line.push(&rest[..start + 1], STRONG);
            rest = tail;
            continue;
        };
        line.push(&rest[..start], STRONG);
        let name = label.strip_prefix("File: ").unwrap_or(label);
        let chip = format!(" {} {name} ", &tail[..digits]).replace(' ', "\u{a0}");
        line.push_on(&chip, LIT, BADGE_IDLE);
        rest = &tail[digits + 2 + label.len() + 1..];
    }
    line.push(rest, STRONG);
    line
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
            } => request(text, columns),
            Item::Text {
                kind: Kind::Assistant,
                text,
            } => prose::render(text, columns),
            Item::Text {
                kind: Kind::Error,
                text,
            } => Line::new(&text::clean(text), BODY)
                .at_source(0)
                .wrap(columns.saturating_sub(2), true)
                .into_iter()
                .enumerate()
                .map(|(index, line)| line.indent(if index == 0 { "✕ " } else { "  " }, BAD.bold()))
                .collect(),
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
