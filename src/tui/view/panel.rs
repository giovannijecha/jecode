//! The menu standard's shared parts: a header with context and aside, a rule
//! or section tabs, inset fields, rows with a scrollbar, a detail strip,
//! keycaps and a confirmation row. `finish` fits them to the panel height.

use crate::tui::{
    line::{Line, Span},
    text,
    theme::{
        ACCENT, BAD, CODE_SURFACE, MUTED, SCANNER_REST, SELECTED_SURFACE, STRONG, Style, Surface,
        USER_SURFACE,
    },
};

pub(super) fn position(start: usize, end: usize, total: usize) -> String {
    if start >= end || start == 0 && end == total {
        String::new()
    } else {
        format!("{}–{end} / {total}", start + 1)
    }
}

fn margin(columns: usize) -> usize {
    if columns >= 12 { 2 } else { 0 }
}

pub(super) fn width(columns: usize) -> usize {
    columns.saturating_sub(2 * margin(columns)).max(1)
}

pub(super) fn row(line: Line, columns: usize) -> Line {
    let mut line = line
        .shortened(width(columns))
        .indent(&" ".repeat(margin(columns)), MUTED);
    line.background.get_or_insert(USER_SURFACE);
    line
}

pub(super) fn cells(line: &Line) -> usize {
    text::cells(&line.plain())
}

/// `line` shortened or padded to exactly `width` cells; padding takes `fill`.
pub(super) fn fit(line: Line, width: usize, fill: Option<Surface>) -> Line {
    let mut line = line.shortened(width);
    let used = cells(&line);
    push(
        &mut line,
        &" ".repeat(width.saturating_sub(used)),
        MUTED,
        fill,
    );
    line
}

pub(super) fn push(line: &mut Line, value: &str, style: Style, background: Option<Surface>) {
    if !value.is_empty() {
        line.spans.push(Span {
            text: value.into(),
            style,
            background,
        });
    }
}

/// `left` and a right-aligned `right` in `width` cells; `left` shortens first.
pub(super) fn spread(left: Line, right: Line, width: usize, fill: Option<Surface>) -> Line {
    let room = cells(&right);
    let gap = usize::from(room > 0);
    let mut line = fit(left, width.saturating_sub(room + gap), fill);
    push(&mut line, &" ".repeat(gap), MUTED, fill);
    line.spans.extend(right.spans);
    fit(line, width, fill)
}

/// Spans without their own surface take `surface`, as a focused row does.
pub(super) fn surfaced(mut line: Line, surface: Surface) -> Line {
    for span in &mut line.spans {
        span.background.get_or_insert(surface);
    }
    line
}

/// `Parent › Title · context` with the aside right-aligned. The context
/// drops first; the aside drops rather than shorten the title below 16 cells.
pub(super) fn header(
    parent: Option<&str>,
    title: &str,
    context: &str,
    aside: Line,
    width: usize,
) -> Line {
    let mut line = Line::default();
    if let Some(parent) = parent {
        line.push(&text::clean(parent), MUTED);
        line.push(" › ", MUTED);
    }
    line.push(&text::clean(title), STRONG.bold());
    let aside = if cells(&line).min(16) + 1 + cells(&aside) <= width {
        aside
    } else {
        Line::default()
    };
    let context = if context.is_empty() {
        String::new()
    } else {
        format!(" · {}", text::clean(context))
    };
    if cells(&line) + text::cells(&context) + cells(&aside) + 2 <= width {
        line.push(&context, MUTED);
    }
    spread(line, aside, width, None)
}

pub(super) fn divider(width: usize) -> Line {
    Line::new(&"─".repeat(width), SCANNER_REST)
}

/// Section names over a rule whose active part is an accent underline;
/// `right` sits at the end of the names row.
pub(super) fn tabs(names: &[&str], active: usize, right: Line, width: usize) -> [Line; 2] {
    let mut strip = Line::default();
    let mut under = Line::default();
    for (index, name) in names.iter().enumerate() {
        if index > 0 {
            strip.push("   ", MUTED);
            under.push("───", SCANNER_REST);
        }
        let on = index == active;
        strip.push(name, if on { STRONG.bold() } else { MUTED });
        under.push(
            &(if on { "━" } else { "─" }).repeat(text::cells(name)),
            if on { ACCENT } else { SCANNER_REST },
        );
    }
    let rest = width.saturating_sub(cells(&under));
    under.push(&"─".repeat(rest), SCANNER_REST);
    [spread(strip, right, width, None), under.shortened(width)]
}

/// An inset input on the code surface. Empty input shows `placeholder`.
pub(super) fn field(value: &str, placeholder: &str, width: usize, style: Style) -> Line {
    let mut line = Line::default();
    push(&mut line, " ", MUTED, Some(CODE_SURFACE));
    if value.is_empty() {
        push(&mut line, placeholder, MUTED, Some(CODE_SURFACE));
    } else {
        push(&mut line, value, style, Some(CODE_SURFACE));
    }
    fit(line, width, Some(CODE_SURFACE))
}

/// The scrollbar cell of visible row `index` for `total` rows shown `limit`
/// at a time from `start`: a `┃` thumb on a `│` track.
pub(super) fn scrollbar(
    index: usize,
    start: usize,
    total: usize,
    limit: usize,
) -> (&'static str, Style) {
    let size = (limit * limit).div_ceil(total).clamp(1, limit);
    let travel = total.saturating_sub(limit).max(1);
    let thumb = (start * (limit - size) + travel / 2) / travel;
    if index >= thumb && index < thumb + size {
        ("┃", MUTED)
    } else {
        ("│", SCANNER_REST)
    }
}

/// `line` followed by its scrollbar cell when the content overflows.
pub(super) fn barred(mut line: Line, bar: Option<(usize, usize, usize, usize)>) -> Line {
    if let Some((index, start, total, limit)) = bar {
        let (glyph, style) = scrollbar(index, start, total, limit);
        line.push(" ", MUTED);
        line.push(glyph, style);
    }
    line
}

/// Two muted rows of prose for the focused item, padded so the panel keeps
/// its height while the focus moves.
pub(super) fn strip(prose: &str, width: usize) -> Vec<Line> {
    let prose = text::clean(prose);
    let mut rows = Line::new(&prose, MUTED).wrap(width, true);
    if rows.len() > 2 {
        let first = rows[0].plain();
        let rest = prose
            .strip_prefix(first.trim_end())
            .unwrap_or(&prose)
            .trim_start();
        rows.truncate(1);
        rows.push(Line::new(&text::ellipsize(rest, width), MUTED));
    }
    rows.resize(2, Line::default());
    rows
}

/// One key hint. Scrolling keys show only when content overflows; optional
/// keys leave first when the row is too narrow.
#[derive(Clone, Copy)]
pub(super) struct Key<'a> {
    pub key: &'a str,
    pub label: &'a str,
    pub scroll: bool,
    pub optional: bool,
}

pub(super) const fn key<'a>(key: &'a str, label: &'a str) -> Key<'a> {
    Key {
        key,
        label,
        scroll: false,
        optional: false,
    }
}

impl Key<'_> {
    pub const fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
    pub const fn scrolling(mut self) -> Self {
        self.scroll = true;
        self
    }
}

pub(super) fn keycap(key: &str) -> Line {
    let mut line = Line::default();
    push(
        &mut line,
        &format!(" {key} "),
        STRONG,
        Some(SELECTED_SURFACE),
    );
    line
}

fn caps(keys: &[Key], label: Style) -> Line {
    let mut line = Line::default();
    for (index, key) in keys.iter().enumerate() {
        if index > 0 {
            line.push("   ", MUTED);
        }
        line.spans.extend(keycap(key.key).spans);
        line.push(&format!(" {}", key.label), label);
    }
    line
}

/// The key row as keycaps; what still does not fit wraps onto more rows.
pub(super) fn keys(keys: &[Key], width: usize, overflow: bool) -> Vec<Line> {
    let mut shown: Vec<Key> = keys
        .iter()
        .copied()
        .filter(|key| overflow || !key.scroll)
        .collect();
    while cells(&caps(&shown, MUTED)) > width {
        let Some(index) = shown.iter().rposition(|key| key.optional) else {
            break;
        };
        shown.remove(index);
    }
    let mut rows: Vec<Vec<Key>> = vec![vec![]];
    for key in shown {
        let last = rows.last_mut().unwrap();
        last.push(key);
        if last.len() > 1 && cells(&caps(last, MUTED)) > width {
            last.pop();
            rows.push(vec![key]);
        }
    }
    rows.iter()
        .map(|row| caps(row, MUTED).shortened(width))
        .collect()
}

/// A destructive step in red with its keys; the question keeps its own row
/// when the keys do not fit beside it.
pub(super) fn confirm(question: &str, keys: &[Key], width: usize) -> Vec<Line> {
    let mut answers = Line::default();
    for (index, key) in keys.iter().enumerate() {
        if index > 0 {
            answers.push("   ", MUTED);
        }
        answers.spans.extend(keycap(key.key).spans);
        answers.push(
            &format!(" {}", key.label),
            if index == 0 { BAD } else { MUTED },
        );
    }
    let mut line = Line::new(question, BAD);
    if cells(&line) + 3 + cells(&answers) <= width {
        line.push("   ", MUTED);
        line.spans.extend(answers.spans);
        vec![line]
    } else {
        vec![line.shortened(width), answers.shortened(width)]
    }
}

/// What a row is, for fitting the panel to a short terminal: the detail
/// strip leaves first, then spacing, then the column header or tabs, then
/// rows far from the focus, then the title. `Keep` rows stay while any row
/// fits; the caret row and a confirmation (`Focus`) stay longest.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Part {
    Focus,
    Keep,
    Title,
    Row(usize),
    Header,
    Space,
    Strip,
}

impl Part {
    fn rank(self) -> usize {
        match self {
            Self::Focus => 0,
            Self::Keep => 1,
            Self::Title => 2,
            Self::Row(distance) => 3 + distance,
            Self::Header => 1 << 20,
            Self::Space => 2 << 20,
            Self::Strip => 3 << 20,
        }
    }
}

/// The panel's rows from the parts, with one padding row above and below.
#[derive(Default)]
pub(super) struct Panel {
    parts: Vec<(Line, Part)>,
    cursor: Option<(usize, usize)>,
}

impl Panel {
    pub fn add(&mut self, line: Line, part: Part) {
        self.parts.push((line, part));
    }
    pub fn space(&mut self) {
        self.add(Line::default(), Part::Space);
    }
    pub fn extend(&mut self, lines: impl IntoIterator<Item = Line>, part: Part) {
        for line in lines {
            self.add(line, part);
        }
    }
    /// The software caret is at `column` of the next row added.
    pub fn caret(&mut self, column: usize) {
        self.cursor = Some((self.parts.len(), column));
    }
    pub fn len(&self) -> usize {
        self.parts.len()
    }
    pub fn finish(self, columns: usize, height: usize) -> (Vec<Line>, Option<(usize, usize)>) {
        if height == 0 {
            return (vec![], None);
        }
        let mut parts = vec![(Line::default(), Part::Space)];
        parts.extend(self.parts);
        parts.push((Line::default(), Part::Space));
        let caret = self.cursor.map(|(row, _)| row + 1);
        let rank = |index: usize| {
            if Some(index) == caret {
                Part::Focus.rank()
            } else {
                parts[index].1.rank()
            }
        };
        let mut order: Vec<usize> = (0..parts.len()).collect();
        order.sort_by_key(|&index| (rank(index), index));
        order.truncate(height);
        // The strip shows whole or not at all.
        let strip = |index: &usize| parts[*index].1 == Part::Strip;
        if order.iter().filter(|index| strip(index)).count()
            < parts.iter().filter(|part| part.1 == Part::Strip).count()
        {
            order.retain(|index| !strip(index));
        }
        order.sort_unstable();
        let cursor = self.cursor.and_then(|(row, column)| {
            order
                .iter()
                .position(|&index| index == row + 1)
                .map(|row| (row, column + margin(columns)))
        });
        let lines = order
            .into_iter()
            .map(|index| row(parts[index].0.clone(), columns))
            .collect();
        (lines, cursor)
    }
}

/// Rows a list may show: at most `limit`, leaving `fixed` rows that always
/// stay. The strip, spacing and headers give way to rows.
pub(super) fn budget(height: usize, fixed: usize, limit: usize) -> usize {
    height.saturating_sub(fixed).clamp(1, limit)
}
