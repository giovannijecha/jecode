mod inline;
mod table;

use super::{
    line::Line,
    syntax::Highlighter,
    text,
    theme::{BODY, CODE_BACKGROUND, EMPHASIS, MUTED},
};

struct Fence {
    marker: u8,
    count: usize,
    highlighter: Highlighter,
}

// An owned display subset. Original messages stay in the conversation/archive.
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
}

impl Rows {
    pub fn new(value: &str, columns: usize) -> Self {
        let mut value = text::clean(value);
        value.truncate(value.trim_end().len());
        Self {
            value,
            position: 0,
            columns: columns.max(2),
            fenced: None,
            table: None,
        }
    }
}

impl Iterator for Rows {
    type Item = Vec<Line>;

    fn next(&mut self) -> Option<Self::Item> {
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
                output.push(Line::default().on(CODE_BACKGROUND).at_source(origin));
                self.fenced = None;
            } else {
                output.extend(
                    fence
                        .highlighter
                        .line(source)
                        .at_source(origin)
                        .wrap(columns, false),
                );
            }
            return Some(output);
        }
        if let Some((marker, count, label)) = opening_fence(source) {
            output.push(
                Line::new(&text::ellipsize(label, columns), MUTED)
                    .on(CODE_BACKGROUND)
                    .at_source(origin),
            );
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
        let heading = trimmed.bytes().take_while(|&byte| byte == b'#').count();
        if horizontal_rule(trimmed) {
            output.push(Line::new(&"─".repeat(columns), MUTED).at_source(origin));
        } else if (1..=6).contains(&heading) && trimmed.as_bytes().get(heading) == Some(&b' ') {
            output.extend(
                inline::render(heading_body(&trimmed[heading + 1..]), EMPHASIS)
                    .at_source(origin)
                    .wrap(columns, true),
            );
        } else if let Some(quote) = trimmed.strip_prefix('>') {
            let quote = quote.strip_prefix(' ').unwrap_or(quote);
            output.extend(prefixed(
                quote,
                &format!("{indentation}│ "),
                columns,
                true,
                MUTED,
                origin,
            ));
        } else if let Some((marker, body)) = list_item(trimmed) {
            let (marker, body) = task_item(marker, body);
            output.extend(prefixed(
                body,
                &format!("{indentation}{marker}"),
                columns,
                false,
                BODY,
                origin,
            ));
        } else {
            if !trimmed.is_empty()
                && let Some((underline, next)) = table::source_line(&self.value, self.position)
                && setext(underline)
            {
                self.position = next;
                output.extend(
                    inline::render(source, EMPHASIS)
                        .at_source(origin)
                        .wrap(columns, true),
                );
            } else {
                output.extend(
                    inline::render(source, BODY)
                        .at_source(origin)
                        .wrap(columns, true),
                );
            }
        }
        Some(output)
    }
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
    prefix: &str,
    columns: usize,
    repeat: bool,
    base: &'static str,
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
                MUTED,
            )
            .wrap(columns, false)
        })
        .collect()
}

#[cfg(test)]
mod tests;
