//! Filterable list ("search"): a query line with a live `N of M` count, the
//! ranked matches with every matched character highlighted, and a key hint.
//! It takes the composer's place in the chrome, framed by the same rules.
//! Short lists skip the filter and become numbered menus (`is_menu`).
//!
//! ```text
//! ────────────────────────────────────────
//!  Model › op▍                     3 of 7
//!  › opus-5.5       most capable
//!    opus-4.7       previous
//! ────────────────────────────────────────
//!  ↑↓ move · Enter select · Esc close
//! ```
use super::block::MARGIN;
use super::glyph::Glyphs;
use super::style::{Row, Tone, pad};
use super::text;
use std::ops::Range;

/// Rows of choices shown at once; the window follows the selection.
pub const VISIBLE: usize = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Choice {
    pub label: String,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Picker {
    pub title: String,
    pub choices: Vec<Choice>,
    pub query: String,
    /// Index into `matches`, not into `choices`.
    pub selected: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Match {
    pub index: usize,
    /// Byte ranges of the label that matched the query.
    pub hits: Vec<Range<usize>>,
}

/// Case-insensitive: substring matches first (earliest position wins), then
/// scattered subsequence matches (tightest span wins); ties keep list order.
pub fn matches(choices: &[Choice], query: &str) -> Vec<Match> {
    let query: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let mut ranked: Vec<(usize, Match)> = choices
        .iter()
        .enumerate()
        .filter_map(|(index, choice)| {
            let (score, hits) = rank(&choice.label, &query)?;
            Some((score, Match { index, hits }))
        })
        .collect();
    ranked.sort_by_key(|(score, found)| (*score, found.index));
    ranked.into_iter().map(|(_, found)| found).collect()
}

fn rank(label: &str, query: &[char]) -> Option<(usize, Vec<Range<usize>>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    let chars: Vec<(usize, char)> = label
        .char_indices()
        .map(|(at, ch)| (at, ch.to_lowercase().next().unwrap_or(ch)))
        .collect();
    let end = |position: usize| chars.get(position).map_or(label.len(), |(at, _)| *at);
    let same =
        |position: usize, wanted: char| chars.get(position).is_some_and(|(_, ch)| *ch == wanted);
    let contiguous = (0..chars.len()).find(|start| {
        query
            .iter()
            .enumerate()
            .all(|(offset, wanted)| same(start + offset, *wanted))
    });
    if let Some(start) = contiguous {
        let hit = chars[start].0..end(start + query.len());
        return Some((start, Vec::from([hit])));
    }
    let mut picked = Vec::with_capacity(query.len());
    let mut position = 0;
    for wanted in query {
        position += (position..chars.len()).position(|at| same(at, *wanted))?;
        picked.push(position);
        position += 1;
    }
    let span = picked[picked.len() - 1] - picked[0];
    let mut hits: Vec<Range<usize>> = Vec::new();
    for position in picked {
        let range = chars[position].0..end(position + 1);
        match hits.last_mut() {
            Some(last) if last.end == range.start => last.end = range.end,
            _ => hits.push(range),
        }
    }
    Some((1_000 + span, hits))
}

/// A list that fits on screen is a numbered menu: no query line, digit keys
/// pick directly. Only longer lists earn a filter.
pub fn is_menu(picker: &Picker) -> bool {
    picker.choices.len() <= VISIBLE
}

/// The choice a digit key picks in a menu, as an index into `choices`.
#[cfg(test)]
pub fn digit(picker: &Picker, key: char) -> Option<usize> {
    let index = (key.to_digit(10)? as usize).checked_sub(1)?;
    (is_menu(picker) && index < picker.choices.len()).then_some(index)
}

pub fn rows(picker: &Picker, width: usize, glyph: &Glyphs) -> Vec<Row> {
    if is_menu(picker) {
        return menu_rows(picker, width, glyph);
    }
    let found = matches(&picker.choices, &picker.query);
    let rule = || Row::new(glyph.rule.repeat(width), Tone::Accent);
    let mut out = vec![rule(), query_row(picker, found.len(), width, glyph)];
    if found.is_empty() {
        out.push(Row::new(format!("{MARGIN}  No matches"), Tone::Muted));
    }
    let selected = picker.selected.min(found.len().saturating_sub(1));
    let start = selected.saturating_sub(VISIBLE - 1);
    let shown = &found[start..found.len().min(start + VISIBLE)];
    let column = shown
        .iter()
        .map(|found| text::width(&picker.choices[found.index].label))
        .max()
        .unwrap_or(0);
    for (offset, found) in shown.iter().enumerate() {
        let is_selected = start + offset == selected;
        out.push(choice_row(
            &picker.choices[found.index],
            &found.hits,
            is_selected,
            column,
            width,
            glyph,
        ));
    }
    let below = found.len() - (start + shown.len());
    if below > 0 {
        out.push(Row::new(
            format!("{MARGIN}  {} {below} more", glyph.ellipsis),
            Tone::Muted,
        ));
    }
    out.push(rule());
    let hint = format!(
        "{} move {1} Enter select {1} Esc close",
        glyph.arrows, glyph.dot
    );
    out.push(Row::new(
        format!(
            "{MARGIN}{}",
            text::clip(&hint, width.saturating_sub(MARGIN.len()), glyph.ellipsis)
        ),
        Tone::Muted,
    ));
    out
}

fn menu_rows(picker: &Picker, width: usize, glyph: &Glyphs) -> Vec<Row> {
    let rule = || Row::new(glyph.rule.repeat(width), Tone::Accent);
    let mut title = Row::new(MARGIN, Tone::Text);
    title.push(
        &text::clip(
            &picker.title,
            width.saturating_sub(MARGIN.len()),
            glyph.ellipsis,
        ),
        Tone::Heading,
    );
    let mut out = vec![rule(), title];
    // The key digit leads the label and is lit like a match.
    let numbered: Vec<Choice> = picker
        .choices
        .iter()
        .enumerate()
        .map(|(index, choice)| Choice {
            label: format!("{}  {}", index + 1, choice.label),
            detail: choice.detail.clone(),
        })
        .collect();
    let column = numbered
        .iter()
        .map(|choice| text::width(&choice.label))
        .max()
        .unwrap_or(0);
    let selected = picker.selected.min(numbered.len().saturating_sub(1));
    for (index, choice) in numbered.iter().enumerate() {
        let digit = 0..1;
        out.push(choice_row(
            choice,
            std::slice::from_ref(&digit),
            index == selected,
            column,
            width,
            glyph,
        ));
    }
    out.push(rule());
    let keys = match picker.choices.len() {
        0 | 1 => "1".to_string(),
        count => format!("1-{count}"),
    };
    let hint = format!(
        "{} move {1} {keys} or Enter select {1} Esc close",
        glyph.arrows, glyph.dot
    );
    out.push(Row::new(
        format!(
            "{MARGIN}{}",
            text::clip(&hint, width.saturating_sub(MARGIN.len()), glyph.ellipsis)
        ),
        Tone::Muted,
    ));
    out
}

fn query_row(picker: &Picker, found: usize, width: usize, glyph: &Glyphs) -> Row {
    let mut row = Row::new(MARGIN, Tone::Text);
    let count = format!("{found} of {}", picker.choices.len());
    let count = text::clip(&count, width.saturating_sub(6), glyph.ellipsis);
    let query_room = usize::from(!picker.query.is_empty()) * 4;
    let title_room =
        width.saturating_sub(MARGIN.len() + 3 + 1 + 1 + text::width(&count) + query_room);
    row.push(
        &text::clip(&picker.title, title_room, glyph.ellipsis),
        Tone::Heading,
    )
    .push(" ", Tone::Text)
    .push(glyph.prompt, Tone::Accent)
    .push(" ", Tone::Text);
    let room = width.saturating_sub(text::width(&row.text) + text::width(&count) + 2);
    // Keep the end of a long query: that is where typing happens.
    let query: Vec<&str> = super::unicode::clusters(&picker.query).collect();
    let mut kept = 0;
    let mut used = 0;
    for cluster in query.iter().rev() {
        used += text::width(cluster);
        if used > room {
            break;
        }
        kept += 1;
    }
    row.push(&query[query.len() - kept..].concat(), Tone::Text)
        .push(" ", Tone::Cursor);
    let gap = width.saturating_sub(text::width(&row.text) + text::width(&count));
    row.push(&" ".repeat(gap), Tone::Text)
        .push(&count, Tone::Muted);
    row
}

/// One list row; shared with the composer's command suggestions.
pub fn choice_row(
    choice: &Choice,
    hits: &[Range<usize>],
    selected: bool,
    column: usize,
    width: usize,
    glyph: &Glyphs,
) -> Row {
    let base = if selected { Tone::User } else { Tone::Text };
    let mut row = Row::new(MARGIN, base);
    let marker = if selected { glyph.prompt } else { " " };
    row.push(marker, Tone::Accent).push(" ", base);
    let label = &choice.label;
    let available = width.saturating_sub(text::width(&row.text));
    let shown = text::clip(label, available, glyph.ellipsis);
    let prefix_len = if text::width(label) <= available {
        label.len()
    } else {
        shown.len().saturating_sub(glyph.ellipsis.len())
    };
    let mut at = 0;
    for hit in hits {
        if hit.start >= prefix_len {
            break;
        }
        let end = hit.end.min(prefix_len);
        row.push(&label[at..hit.start], base)
            .push(&label[hit.start..end], Tone::Accent);
        at = end;
    }
    row.push(&label[at..prefix_len], base);
    if prefix_len < label.len() {
        row.push(glyph.ellipsis, base);
    }
    if !choice.detail.is_empty() {
        let gap = column.saturating_sub(text::width(label)) + 3;
        let room = width.saturating_sub(text::width(&row.text) + gap + MARGIN.len());
        if room > 4 {
            row.push(&" ".repeat(gap), base).push(
                &text::clip(&choice.detail, room, glyph.ellipsis),
                Tone::Muted,
            );
        }
    }
    if selected { pad(row, width) } else { row }
}

#[cfg(test)]
#[path = "picker_tests.rs"]
mod tests;
