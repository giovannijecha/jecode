#[cfg(test)]
use super::State;
use super::{Item, Kind};
use crate::{
    attachments::MARKER,
    tui::{
        cards,
        line::Line,
        prose,
        references::syntax,
        text,
        theme::{ACCENT, BAD, BADGE_IDLE, BODY, LIT, STRONG, USER_SURFACE},
    },
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

#[derive(Default, Clone, Copy, PartialEq, Eq)]
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

/// The sent request: `› ` on one band of the full-row user surface, the text hanging
/// at column 2 and attachment labels and references drawn as chips.
fn request(value: &str, columns: usize) -> Vec<Line> {
    let width = columns.saturating_sub(4);
    let Chips {
        line,
        positions,
        source,
        ..
    } = chips(&text::clean(value), width);
    let mut line = line.at_source(0);
    line.background = Some(USER_SURFACE);
    line.wrap(width, true)
        .into_iter()
        .enumerate()
        .map(|(index, mut line)| {
            if let Some(origin) = &mut line.origin {
                origin.character = source.original(positions.original(origin.character));
            }
            line.indent(if index == 0 { "› " } else { "  " }, ACCENT.bold())
        })
        .collect()
}

/// `[1# Image]` and `[2# File: wake.log]` become ` 1 Image ` and ` 2 wake.log `, and
/// the references the composer draws become ` $review ` and ` @AGENTS.md `, in LIT
/// on the quiet badge surface. Inner spaces never break; a reference wider than
/// `columns` is shortened on screen only, and chips that touch keep one plain cell
/// between them so wrapping can separate them.
fn chips(value: &str, columns: usize) -> Chips {
    // Each label stands for its attachment marker, a reference boundary as in the composer.
    let mut source = String::with_capacity(value.len());
    let mut positions = Positions::default();
    let mut labels = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find('[') {
        let tail = &rest[start + 1..];
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        let label = tail[digits..]
            .strip_prefix("# ")
            .and_then(|label| label.find(']').map(|end| &label[..end]))
            .filter(|label| digits > 0 && !label.is_empty());
        let Some(label) = label else {
            source.push_str(&rest[..start + 1]);
            let length = rest[..start + 1].chars().count();
            positions.advance(length, length);
            rest = tail;
            continue;
        };
        source.push_str(&rest[..start]);
        let length = rest[..start].chars().count();
        positions.advance(length, length);
        source.push(MARKER);
        let name = label.strip_prefix("File: ").unwrap_or(label);
        labels.push(format!(" {} {name} ", &tail[..digits]));
        let end = digits + 2 + label.len() + 1;
        positions.advance(1, rest[start..start + 1 + end].chars().count());
        rest = &tail[end..];
    }
    source.push_str(rest);
    let length = rest.chars().count();
    positions.advance(length, length);
    let mut chips = Chips {
        source: positions,
        ..Chips::default()
    };
    let mut labels = labels.into_iter();
    let mut text = |chips: &mut Chips, value: &str| {
        for (index, part) in value.split(MARKER).enumerate() {
            if index > 0 {
                chips.chip(&labels.next().unwrap_or_default(), 1);
            }
            chips.text(part);
        }
    };
    let mut at = 0;
    for reference in syntax::chips(&source) {
        text(&mut chips, &source[at..reference.start]);
        let name = text::ellipsize(&source[reference.clone()], columns.saturating_sub(2).max(1));
        chips.chip(
            &format!(" {name} "),
            source[reference.clone()].chars().count(),
        );
        at = reference.end;
    }
    text(&mut chips, &source[at..]);
    chips
}

#[derive(Default)]
struct Chips {
    line: Line,
    touching: bool,
    positions: Positions,
    source: Positions,
}

impl Chips {
    fn text(&mut self, value: &str) {
        self.line.push(value, STRONG);
        let length = value.chars().count();
        self.positions.advance(length, length);
        self.touching &= value.is_empty();
    }

    fn chip(&mut self, value: &str, source_length: usize) {
        if self.touching {
            self.line.push(" ", STRONG);
            self.positions.advance(1, 0);
        }
        self.line
            .push_on(&value.replace(' ', "\u{a0}"), LIT, BADGE_IDLE);
        self.positions.advance(value.chars().count(), source_length);
        self.touching = true;
    }
}

/// Source character positions after display-only insertions and replacements.
/// Only changes in length need an entry; ordinary prose keeps a linear mapping.
#[derive(Default)]
struct Positions {
    display: usize,
    source: usize,
    changes: Vec<(usize, usize)>,
}

impl Positions {
    fn advance(&mut self, display: usize, source: usize) {
        self.display += display;
        self.source += source;
        if display != source {
            self.changes.push((self.display, self.source));
        }
    }

    fn original(&self, character: usize) -> usize {
        let index = self
            .changes
            .partition_point(|(display, _)| *display <= character);
        let (display, source) = index
            .checked_sub(1)
            .map_or((0, 0), |index| self.changes[index]);
        source + character - display
    }
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
