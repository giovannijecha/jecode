mod inline;
mod stream;
mod table;

pub(super) use stream::Stream;

use super::{
    line::{Fold, Line},
    syntax::Highlighter,
    text,
    theme::{ACCENT, BODY, CODE_SURFACE, CODE_TEXT, FAINT, GOOD, MUTED, STRONG, Style},
};
use std::borrow::Cow;

#[derive(Clone)]
struct Fence {
    marker: u8,
    count: usize,
    highlighter: Highlighter,
}

// An owned display subset. Original messages stay in the conversation/archive.
#[cfg(test)]
pub fn render(value: &str, columns: usize) -> Vec<Line> {
    Rows::new(value, columns).flatten().collect()
}

// Source lines can be laid out across event-loop passes. Fence/highlighter state
// belongs to this iterator, not to a terminal frame or the provider conversation.
pub(super) struct Rows {
    value: String,
    position: usize,
    columns: usize,
    fenced: Option<Fence>,
    table: Option<table::Table>,
    empty_pending: bool,
}

impl Rows {
    pub fn new(value: &str, columns: usize) -> Self {
        let mut value = text::clean(value);
        value.truncate(value.trim_end().len());
        Self::from_clean(value, columns)
    }

    fn from_clean(value: String, columns: usize) -> Self {
        let empty_pending = value.is_empty();
        Self {
            value,
            position: 0,
            columns: columns.max(2),
            fenced: None,
            table: None,
            empty_pending,
        }
    }
}

impl Iterator for Rows {
    type Item = Vec<Line>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.empty_pending {
            self.empty_pending = false;
            return Some(vec![Line::default().at_source(0)]);
        }
        if let Some(table) = &mut self.table {
            if let Some(lines) = table.step(&self.value) {
                return Some(lines);
            }
            self.position = table.end();
            self.table = None;
        }
        if self.position >= self.value.len() {
            return None;
        }
        let end = self.value[self.position..]
            .find('\n')
            .map_or(self.value.len(), |offset| self.position + offset);
        let origin = self.position;
        let source = &self.value[self.position..end];
        self.position = end + 1;
        let columns = self.columns;
        let mut output = Vec::new();
        if let Some(fence) = &mut self.fenced {
            let trimmed = source.trim_start();
            let count = trimmed
                .bytes()
                .take_while(|&byte| byte == fence.marker)
                .count();
            if count >= fence.count && trimmed[count..].trim().is_empty() {
                output.push(Line::default().on(CODE_SURFACE).at_source(origin));
                self.fenced = None;
            } else {
                let fold = Fold {
                    markup: fence.highlighter.markup(),
                    ..Fold::default()
                };
                output.extend(
                    fence
                        .highlighter
                        .line(source)
                        .at_source(origin)
                        .code(padded(columns), fold)
                        .into_iter()
                        .map(|row| pad(row, columns)),
                );
            }
            return Some(output);
        }
        if let Some((marker, count, label)) = opening_fence(source) {
            output.push(pad(
                Line::new(&text::ellipsize(label, padded(columns)), MUTED)
                    .on(CODE_SURFACE)
                    .at_source(origin),
                columns,
            ));
            self.fenced = Some(Fence {
                marker,
                count,
                highlighter: Highlighter::new(label),
            });
            return Some(output);
        }
        if let Some((delimiter, body)) = table::source_line(&self.value, self.position)
            && let Some(table) = table::Table::new(source, delimiter, body, columns, origin)
        {
            self.table = Some(table);
            return Some(Vec::new());
        }
        let trimmed = source.trim_start();
        let indentation = &source[..source.len() - trimmed.len()];
        let level = trimmed.bytes().take_while(|&byte| byte == b'#').count();
        if horizontal_rule(trimmed) {
            output.push(Line::new(&"─".repeat(columns), FAINT).at_source(origin));
        } else if (1..=6).contains(&level) && trimmed.as_bytes().get(level) == Some(&b' ') {
            let body = heading_body(&trimmed[level + 1..]);
            output.extend(heading(
                body,
                level,
                origin,
                columns,
                &self.value,
                self.position,
            ));
        } else if let Some(quote) = quoted(trimmed) {
            let quote = joined(&self.value, quote, &mut self.position, columns, true, false);
            output.extend(prefixed(
                &quote,
                (&format!("{indentation}│ "), MUTED),
                columns,
                true,
                MUTED,
                origin,
            ));
        } else if let Some((marker, body)) = list_item(trimmed) {
            let (marker, body) = task_item(marker, body);
            let body = joined(&self.value, body, &mut self.position, columns, false, false);
            output.extend(prefixed(
                &body,
                (
                    &format!("{indentation}{marker}"),
                    if marker == "☑ " { GOOD } else { MUTED },
                ),
                columns,
                false,
                BODY,
                origin,
            ));
        } else {
            let source = if trimmed.is_empty() {
                Cow::Borrowed(source)
            } else {
                joined(
                    &self.value,
                    source,
                    &mut self.position,
                    columns,
                    false,
                    true,
                )
            };
            if !trimmed.is_empty()
                && let Some((underline, next)) = table::source_line(&self.value, self.position)
                && setext(underline)
            {
                self.position = next;
                let level = if underline.trim_start().starts_with('=') {
                    1
                } else {
                    2
                };
                output.extend(heading(
                    source.trim_start(),
                    level,
                    origin,
                    columns,
                    &self.value,
                    next,
                ));
            } else {
                output.extend(
                    inline::render(&source, BODY)
                        .at_source(origin)
                        .wrap(columns, true),
                );
            }
        }
        Some(output)
    }
}

// A heading is bold in its level's colour and keeps one neutral row before
// the text it introduces.
fn heading(
    body: &str,
    level: usize,
    origin: usize,
    columns: usize,
    value: &str,
    next: usize,
) -> Vec<Line> {
    let colour = match level {
        1 => ACCENT,
        2 => STRONG,
        _ => BODY,
    };
    let mut rows = inline::heading(body, colour)
        .at_source(origin)
        .wrap(columns, true);
    if table::source_line(value, next).is_some_and(|(line, _)| !line.trim().is_empty()) {
        rows.push(Line::default());
    }
    rows
}

// CommonMark soft breaks: plain lines continuing a paragraph, list item or
// quote join it with one space, so emphasis can span them and the text reflows
// at any width. A quote also continues through `>` lines. A streamed block is
// laid out again on each append, so a very long one breaks at a source line.
fn joined<'a>(
    value: &'a str,
    first: &'a str,
    position: &mut usize,
    columns: usize,
    quote: bool,
    setext_body: bool,
) -> Cow<'a, str> {
    const LIMIT: usize = 8 * 1024;
    let mut text = Cow::Borrowed(first);
    if quote && first.trim().is_empty() {
        return text;
    }
    // Two trailing spaces or an unescaped backslash end the line with a hard break.
    while text.len() < LIMIT && !text.ends_with("  ") && !backslash_break(&text) {
        let inner = quote
            .then(|| quote_continuation(value, *position))
            .flatten();
        let Some((line, next)) =
            inner.or_else(|| continuation(value, *position, columns, setext_body))
        else {
            return text;
        };
        let joined = text.to_mut();
        joined.truncate(joined.trim_end().len());
        joined.push(' ');
        joined.push_str(line);
        *position = next;
    }
    // A trailing backslash is a hard break only inside the same paragraph.
    // At a block boundary (including a blank line), it remains literal.
    if backslash_break(&text)
        && (quote && quote_continuation(value, *position).is_some()
            || continuation(value, *position, columns, setext_body).is_some())
    {
        text.to_mut().pop();
    }
    text
}

fn backslash_break(value: &str) -> bool {
    value
        .bytes()
        .rev()
        .take_while(|&byte| byte == b'\\')
        .count()
        % 2
        == 1
}

fn continuation(
    value: &str,
    position: usize,
    columns: usize,
    setext_body: bool,
) -> Option<(&str, usize)> {
    let (line, next) = table::source_line(value, position)?;
    // A table header stays separate. Prose may include a line before a setext
    // underline; the heading is rendered from the whole joined paragraph.
    let own = table::source_line(value, next).is_some_and(|(below, after)| {
        (!setext_body && setext(below))
            || table::Table::new(line, below, after, columns, 0).is_some()
    });
    (!line.trim().is_empty() && !setext(line) && !table::block_start(line) && !own)
        .then(|| (line.trim_start(), next))
}

fn quote_continuation(value: &str, position: usize) -> Option<(&str, usize)> {
    let (line, next) = table::source_line(value, position)?;
    let inner = quoted(line.trim_start())?.trim_start();
    (!inner.is_empty() && !table::block_start(inner)).then_some((inner, next))
}

// Keep padding only when the inner width can hold a two-cell glyph.
fn padded(columns: usize) -> usize {
    if columns < 4 { columns } else { columns - 2 }
}

fn pad(row: Line, columns: usize) -> Line {
    if row.spans.is_empty() || columns < 4 {
        row
    } else {
        row.indent(" ", CODE_TEXT)
    }
}

fn quoted(value: &str) -> Option<&str> {
    let quote = value.strip_prefix('>')?;
    Some(quote.strip_prefix(' ').unwrap_or(quote))
}

fn heading_body(value: &str) -> &str {
    let value = value.trim_end();
    let body = value.trim_end_matches('#');
    if body.len() < value.len() && body.ends_with([' ', '\t']) {
        body.trim_end()
    } else {
        value
    }
}

fn setext(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && (value.bytes().all(|byte| byte == b'=') || value.bytes().all(|byte| byte == b'-'))
}

fn task_item<'a>(marker: &'a str, body: &'a str) -> (&'a str, &'a str) {
    if let Some(tail) = body.strip_prefix("[ ] ") {
        ("☐ ", tail)
    } else if let Some(tail) = body
        .strip_prefix("[x] ")
        .or_else(|| body.strip_prefix("[X] "))
    {
        ("☑ ", tail)
    } else {
        (marker, body)
    }
}

fn horizontal_rule(value: &str) -> bool {
    let mut markers = value.chars().filter(|character| *character != ' ');
    let Some(marker @ ('-' | '*' | '_')) = markers.next() else {
        return false;
    };
    let mut count = 1;
    for character in markers {
        if character != marker {
            return false;
        }
        count += 1;
    }
    count >= 3
}

fn opening_fence(value: &str) -> Option<(u8, usize, &str)> {
    let value = value.trim_start();
    let marker = *value.as_bytes().first()?;
    if !matches!(marker, b'\x60' | b'~') {
        return None;
    }
    let count = value.bytes().take_while(|&byte| byte == marker).count();
    if count < 3 {
        return None;
    }
    let label = value[count..].trim();
    if marker == b'\x60' && label.contains('\u{0060}') {
        return None;
    }
    Some((marker, count, label))
}

fn list_item(value: &str) -> Option<(&str, &str)> {
    if ["- ", "* ", "+ "]
        .iter()
        .any(|marker| value.starts_with(marker))
    {
        return Some(("• ", &value[2..]));
    }
    let digits = value.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || !matches!(value.as_bytes().get(digits), Some(b'.' | b')')) {
        return None;
    }
    let tail = value.get(digits + 1..)?;
    let body = tail.strip_prefix(' ')?;
    Some((&value[..digits + 2], body))
}

fn prefixed(
    value: &str,
    (prefix, marker): (&str, Style),
    columns: usize,
    repeat: bool,
    base: Style,
    origin: usize,
) -> Vec<Line> {
    let width = text::cells(prefix);
    let available = columns.saturating_sub(width).max(2);
    let continuation = " ".repeat(width);
    inline::render(value, base)
        .at_source(origin)
        .wrap(available, true)
        .into_iter()
        .enumerate()
        .flat_map(|(index, line)| {
            line.indent(
                if index == 0 || repeat {
                    prefix
                } else {
                    &continuation
                },
                marker,
            )
            .wrap(columns, false)
        })
        .collect()
}

#[cfg(test)]
mod code_tests;
#[cfg(test)]
mod tests;
