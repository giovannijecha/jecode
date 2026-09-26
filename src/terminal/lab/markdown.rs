//! Small owned Markdown subset for assistant prose: headings, paragraphs,
//! bullet and numbered lists with hanging indents, quotes, rules, inline
//! bold / code / links, and fenced code panels. Line-local by design: only
//! the fence flag crosses lines, so streaming re-renders just the open tail.
use super::glyph::Glyphs;
use super::style::{Row, Tone, pad};
use super::text;
use std::ops::Range;

type Spans = Vec<(Range<usize>, Tone)>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct State {
    code: bool,
}

impl State {
    /// Append the rows for one sanitized source line at `width` cells.
    pub fn line(&mut self, line: &str, width: usize, glyph: &Glyphs, out: &mut Vec<Row>) {
        if let Some(language) = line.trim_start().strip_prefix("```") {
            let mut edge = Row::new(" ", Tone::Code);
            if !self.code {
                edge.push(language.trim(), Tone::Muted);
            }
            out.push(pad(edge, width));
            self.code = !self.code;
        } else if self.code {
            code(line, width, out);
        } else {
            prose(line, width, glyph, out);
        }
    }
}

fn code(line: &str, width: usize, out: &mut Vec<Row>) {
    let inner = width.saturating_sub(2).max(1);
    for range in text::wrap_line(line, inner) {
        let mut row = Row::new(format!(" {}", &line[range]), Tone::Code);
        highlight(&mut row);
        out.push(pad(row, width));
    }
}

fn prose(line: &str, width: usize, glyph: &Glyphs, out: &mut Vec<Row>) {
    let body = line.trim_start();
    // Two source spaces per nesting level, two display cells per level.
    let nest = " ".repeat((line.len() - body.len()) / 2 * 2);
    if body.is_empty() {
        out.push(Row::blank());
    } else if let Some(title) = heading(body) {
        flow(out, title, width, Row::new("", Tone::Heading), false);
    } else if rule(body) {
        out.push(Row::new(glyph.rule.repeat(width), Tone::Muted));
    } else if let Some(quote) = body.strip_prefix("> ").or((body == ">").then_some("")) {
        let mut lead = Row::new("", Tone::Muted);
        lead.push(glyph.stem, Tone::Muted).push(" ", Tone::Muted);
        flow(out, quote, width, lead, true);
    } else if let Some(item) = ["- ", "* ", "+ "].iter().find_map(|m| body.strip_prefix(m)) {
        let mut lead = Row::new(nest, Tone::Text);
        lead.push(glyph.bullet, Tone::Muted).push(" ", Tone::Text);
        flow(out, item, width, lead, false);
    } else if let Some((number, item)) = ordered(body) {
        let mut lead = Row::new(nest, Tone::Text);
        lead.push(number, Tone::Muted).push(" ", Tone::Text);
        flow(out, item, width, lead, false);
    } else {
        flow(out, body, width, Row::new("", Tone::Text), false);
    }
}

fn heading(body: &str) -> Option<&str> {
    let title = body.trim_start_matches('#');
    let level = body.len() - title.len();
    (1..=6)
        .contains(&level)
        .then(|| title.strip_prefix(' '))
        .flatten()
}

fn rule(body: &str) -> bool {
    let marks: String = body.chars().filter(|ch| *ch != ' ').collect();
    marks.len() >= 3
        && ['-', '*', '_']
            .iter()
            .any(|mark| marks.chars().all(|ch| ch == *mark))
}

fn ordered(body: &str) -> Option<(&str, &str)> {
    let digits = body.bytes().take_while(u8::is_ascii_digit).count();
    let rest = body[digits..].strip_prefix(". ")?;
    (1..=3)
        .contains(&digits)
        .then(|| (&body[..digits + 1], rest))
}

/// Wrap `body` after `lead`. Continuation rows repeat the lead (quotes) or
/// hang under its width (lists), so wrapped text keeps its column.
fn flow(out: &mut Vec<Row>, body: &str, width: usize, lead: Row, repeat: bool) {
    let hang = text::width(&lead.text);
    let tone = lead.tone;
    let (plain, spans) = inline(body);
    let ranges = text::wrap_line(&plain, width.saturating_sub(hang).max(1));
    for (index, range) in ranges.into_iter().enumerate() {
        let mut row = if index == 0 || repeat {
            lead.clone()
        } else {
            Row::new(" ".repeat(hang), tone)
        };
        let mut at = range.start;
        for (span, span_tone) in &spans {
            let (from, to) = (span.start.max(range.start), span.end.min(range.end));
            if from < to {
                row.push(&plain[at..from], tone)
                    .push(&plain[from..to], *span_tone);
                at = to;
            }
        }
        row.push(&plain[at..range.end], tone);
        out.push(row);
    }
}

/// `**bold**`, `` `code` `` and `[label](url)`; everything else is literal.
fn inline(input: &str) -> (String, Spans) {
    let mut plain = String::new();
    let mut spans = Vec::new();
    let mut rest = input;
    let mut mark = |plain: &mut String, piece: &str, tone: Tone| {
        let start = plain.len();
        plain.push_str(piece);
        spans.push((start..plain.len(), tone));
    };
    while let Some(ch) = rest.chars().next() {
        if let Some((inner, after)) = delimited(rest, "**") {
            mark(&mut plain, inner, Tone::Heading);
            rest = after;
        } else if let Some((inner, after)) = delimited(rest, "`") {
            mark(&mut plain, inner, Tone::Accent);
            rest = after;
        } else if let Some((label, url, after)) = link(rest) {
            mark(&mut plain, label, Tone::Accent);
            if url != label {
                mark(&mut plain, &format!(" ({url})"), Tone::Muted);
            }
            rest = after;
        } else {
            plain.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    (plain, spans)
}

fn delimited<'a>(rest: &'a str, marker: &str) -> Option<(&'a str, &'a str)> {
    let body = rest.strip_prefix(marker)?;
    let end = body.find(marker).filter(|end| *end > 0)?;
    Some((&body[..end], &body[end + marker.len()..]))
}

fn link(rest: &str) -> Option<(&str, &str, &str)> {
    let body = rest.strip_prefix('[')?;
    let close = body.find("](")?;
    let tail = &body[close + 2..];
    let end = tail.find(')')?;
    Some((&body[..close], &tail[..end], &tail[end + 1..]))
}

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "class", "const", "continue", "def", "else", "enum", "false",
    "fn", "for", "from", "function", "if", "impl", "import", "in", "let", "loop", "match", "mod",
    "mut", "None", "pub", "return", "self", "Self", "Some", "struct", "trait", "true", "type",
    "use", "where", "while",
];

/// Tiny lexical highlighter: `//` comments, "strings", keywords, numbers.
fn highlight(row: &mut Row) {
    let text = row.text.as_str();
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let start = index;
        let tone = if bytes[index..].starts_with(b"//") {
            index = bytes.len();
            Some(Tone::Muted)
        } else if bytes[index] == b'"' {
            index += 1;
            while index < bytes.len() && bytes[index] != b'"' {
                index += if bytes[index] == b'\\' { 2 } else { 1 };
            }
            index = (index + 1).min(bytes.len());
            Some(Tone::String)
        } else if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            KEYWORDS
                .contains(&&text[start..index])
                .then_some(Tone::Keyword)
        } else if bytes[index].is_ascii_digit() {
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'.')
            {
                index += 1;
            }
            Some(Tone::Number)
        } else {
            index += text[index..].chars().next().map_or(1, char::len_utf8);
            None
        };
        // Escapes may step past a multi-byte char; realign before slicing.
        while !text.is_char_boundary(index) {
            index += 1;
        }
        if let Some(tone) = tone {
            spans.push((start..index, tone));
        }
    }
    row.spans = spans;
}

#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;
