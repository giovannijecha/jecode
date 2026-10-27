//! The panel grammar: a panel opens below the composer in place of the
//! footer. Its title and aside ride the composer's bottom rule, typed input
//! stays in the composer, and its rows follow: tabs or a column header, a
//! list or text, one blank row, then keycaps or a confirmation.

use crate::tui::{
    line::{Line, Span},
    text,
    theme::{ACCENT, BAD, FAINT, MUTED, SELECTED_SURFACE, STRONG, Style, Surface},
};

pub(super) fn position(start: usize, end: usize, total: usize) -> String {
    if start >= end || start == 0 && end == total {
        String::new()
    } else {
        format!("{}–{end} / {total}", start + 1)
    }
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
    let gap = usize::from(room > 0) * 2;
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

/// Cells for text between column 2 and two cells before the edge; below
/// seven columns, as `indent` does, the whole row.
pub(super) fn text_width(columns: usize) -> usize {
    if columns < 7 {
        columns.max(1)
    } else {
        columns - 4
    }
}

/// A text row at column 2, ending two cells before the edge.
pub(super) fn indent(line: Line, columns: usize) -> Line {
    if columns < 7 {
        return line.shortened(columns);
    }
    line.shortened(text_width(columns)).indent("  ", MUTED)
}

/// `Parent › Title · context` for the rule, then without the context.
pub(super) fn title(parent: Option<&str>, title: &str, context: &str) -> Vec<Line> {
    let mut line = Line::default();
    if let Some(parent) = parent {
        line.push(&text::clean(parent), MUTED);
        line.push(" › ", MUTED);
    }
    line.push(&text::clean(title), STRONG.bold());
    if context.is_empty() {
        return vec![line];
    }
    let mut full = line.clone();
    full.push(&format!(" · {}", text::clean(context)), MUTED);
    vec![full, line]
}

/// Section names at column 2, the active one bold over an accent `━` on a
/// faint divider to the edge; `mode` ends two cells before the edge.
pub(super) fn tabs(names: &[&str], active: usize, mode: Line, columns: usize) -> [Line; 2] {
    let mut strip = Line::default();
    let mut under = Line::new("  ", MUTED);
    for (index, name) in names.iter().enumerate() {
        if index > 0 {
            strip.push("   ", MUTED);
            under.push("───", FAINT);
        }
        let on = index == active;
        strip.push(name, if on { STRONG.bold() } else { MUTED });
        under.push(
            &(if on { "━" } else { "─" }).repeat(text::cells(name)),
            if on { ACCENT } else { FAINT },
        );
    }
    let rest = columns.saturating_sub(cells(&under));
    under.push(&"─".repeat(rest), FAINT);
    let strip = spread(strip, mode, text_width(columns), None);
    [indent(strip, columns), under.shortened(columns)]
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
        ("│", FAINT)
    }
}

/// The first row of a `room`-row window over `total` rows centred on `focus`.
pub(super) fn start(focus: usize, total: usize, room: usize) -> usize {
    focus
        .saturating_sub(room / 2)
        .min(total.saturating_sub(room))
}

/// Rows `start..` of `total`, each padded to the edge, or to its scrollbar
/// in the last column when the rows overflow `room`. A row's surface fills
/// it up to the scrollbar.
pub(super) fn window(
    rows: Vec<(Line, Option<Surface>)>,
    start: usize,
    total: usize,
    room: usize,
    columns: usize,
) -> Vec<Line> {
    let overflow = total > room && columns >= 7;
    rows.into_iter()
        .enumerate()
        .map(|(index, (row, fill))| {
            let mut row = fit(row, columns - usize::from(overflow), fill);
            if let Some(fill) = fill {
                row = surfaced(row, fill);
            }
            if overflow {
                let (glyph, style) = scrollbar(index, start, total, room);
                row.push(glyph, style);
            }
            row
        })
        .collect()
}

/// An option row for `window`: `content` from column 0 ends two cells
/// before the edge; the focused row takes the selected surface.
pub(super) fn choice(content: Line, on: bool, columns: usize) -> (Line, Option<Surface>) {
    (
        fit(content, columns.saturating_sub(2), None),
        on.then_some(SELECTED_SURFACE),
    )
}

/// `› ` on the focused row, in `bad` for a pending delete.
pub(super) fn marker(on: bool, deleting: bool) -> Line {
    Line::new(
        if on { "› " } else { "  " },
        if deleting { BAD } else { ACCENT }.bold(),
    )
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

fn caps(keys: &[Key], first: Style) -> Line {
    let mut line = Line::default();
    for (index, key) in keys.iter().enumerate() {
        if index > 0 {
            line.push("   ", MUTED);
        }
        line.spans.extend(keycap(key.key).spans);
        line.push(
            &format!(" {}", key.label),
            if index == 0 { first } else { MUTED },
        );
    }
    line
}

/// One row of keycaps; keys leave from the right when it is too narrow.
pub(super) fn row(keys: &[Key], width: usize) -> Line {
    let mut shown = keys.len();
    while shown > 1 && cells(&caps(&keys[..shown], MUTED)) > width {
        shown -= 1;
    }
    caps(&keys[..shown], MUTED).shortened(width)
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

/// A destructive step: the question in `bad` replaces the keys, which follow
/// on its row or wrap below it.
pub(super) fn confirm(question: &str, keys: &[Key], width: usize) -> Vec<Line> {
    let answers = caps(keys, BAD);
    let mut line = Line::new(question, BAD);
    if cells(&line) + 3 + cells(&answers) <= width {
        line.push("   ", MUTED);
        line.spans.extend(answers.spans);
        vec![line]
    } else {
        vec![line.shortened(width), answers.shortened(width)]
    }
}

/// Text typed in the composer for a panel: its top rule names the field.
pub(super) struct Field {
    pub label: String,
    /// The visible value, one dot per character for a secret.
    pub value: String,
    /// The caret's cell within `value`.
    pub caret: usize,
    pub placeholder: &'static str,
    /// Muted text after the value, such as a unit.
    pub suffix: &'static str,
}

/// Body rows for a room of `rows`, each with its part.
pub(super) type Body<'a> = Box<dyn Fn(usize) -> Vec<(Line, Part)> + 'a>;

/// What a panel puts in the dock below the status row.
pub(super) struct Sheet<'a> {
    /// The rule title, longest first; later entries drop detail.
    pub title: Vec<Line>,
    pub aside: Line,
    /// The composer shows the draft when no field takes its place; the
    /// draft keeps its caret only when it is the panel's filter.
    pub field: Option<Field>,
    pub draft_caret: bool,
    /// Tabs or a column header, full width.
    pub head: Vec<Line>,
    pub body: Body<'a>,
    /// Rows a scrolling body holds; its keys depend on whether they overflow.
    pub total: Option<usize>,
    /// The keys or the confirmation at the text width, for an overflowing
    /// body or one that fits; the first is never shorter.
    pub tail: Box<dyn Fn(bool) -> Vec<Line> + 'a>,
    pub confirming: bool,
}

/// What a row is, for fitting the dock to a short terminal: spacing leaves
/// first, then extra draft rows, the column header or tabs, the status row
/// and the top rule, then rows far from the focus. `Keep` rows stay while
/// any row fits; the caret row and a confirmation (`Focus`) stay longest.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Part {
    Focus,
    Keep,
    Row(usize),
    Chrome,
    Header,
    Draft,
    Space,
}

impl Part {
    fn rank(self) -> usize {
        match self {
            Self::Focus => 0,
            Self::Keep => 1,
            Self::Row(distance) => 2 + distance,
            Self::Chrome => 1 << 20,
            Self::Header => 2 << 20,
            Self::Draft => 3 << 20,
            Self::Space => 4 << 20,
        }
    }
}

/// The parts that fit `height`, in order; `caret` names a part's row and
/// column and ranks that row as the focus.
pub(super) fn kept_indices(
    parts: &[(Line, Part)],
    caret: Option<(usize, usize)>,
    height: usize,
) -> Vec<usize> {
    let rank = |index: usize| {
        if caret.is_some_and(|(row, _)| row == index) {
            Part::Focus.rank()
        } else {
            parts[index].1.rank()
        }
    };
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by_key(|&index| (rank(index), index));
    order.truncate(height);
    order.sort_unstable();
    order
}

pub(super) fn fitted(
    parts: Vec<(Line, Part)>,
    caret: Option<(usize, usize)>,
    height: usize,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let order = kept_indices(&parts, caret, height);
    let cursor = caret.and_then(|(row, column)| {
        order
            .iter()
            .position(|&index| index == row)
            .map(|row| (row, column))
    });
    let mut parts: Vec<Option<Line>> = parts.into_iter().map(|(line, _)| Some(line)).collect();
    let lines = order
        .into_iter()
        .map(|index| parts[index].take().unwrap())
        .collect();
    (lines, cursor)
}

/// Rows a panel body may show: a third of the screen, six to twelve rows,
/// within the two-thirds cap after `fixed` rows, and never under three.
pub(super) fn room(screen: usize, height: usize, fixed: usize) -> usize {
    (screen / 3)
        .clamp(6, 12)
        .min(height.saturating_sub(fixed))
        .max(3)
}
