//! Sanitising, wrapping, clipping and padding. Every operation walks grapheme
//! clusters from `unicode`, so no layout ever splits a letter or an emoji.
use super::unicode::{boundaries, cluster_width, clusters};
use std::ops::Range;

pub use super::unicode::width;

/// Strip anything that could steer the terminal: controls become `?`, bidi
/// overrides become `?`, CRLF and lone CR become LF, tabs become four spaces.
pub fn safe(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' if chars.peek() == Some(&'\n') => {}
            '\r' | '\n' => out.push('\n'),
            '\t' => out.push_str("    "),
            ch if ch.is_control() || bidi(ch) => out.push('?'),
            ch => out.push(ch),
        }
    }
    out
}

fn bidi(ch: char) -> bool {
    matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Word-wrap one sanitized line to `columns` cells, as byte ranges into it.
/// Breaks at the last space when the remainder fits, otherwise between
/// clusters; seam spaces belong to no row. A lone cluster wider than
/// `columns` gets its own row rather than being split.
pub fn wrap_line(line: &str, columns: usize) -> Vec<Range<usize>> {
    let columns = columns.max(1);
    let mut rows = Vec::new();
    let (mut start, mut end, mut used) = (0, 0, 0);
    let mut seam = None;
    for pair in boundaries(line).windows(2) {
        let (at, next) = (pair[0], pair[1]);
        let unit = &line[at..next];
        let size = cluster_width(unit);
        if used + size > columns && end > start {
            if unit == " " {
                rows.push(start..end);
                (start, end, used, seam) = (next, next, 0, None);
                continue;
            }
            match seam {
                Some(space)
                    if !line[start..space].trim().is_empty()
                        && width(&line[space + 1..end]) + size <= columns =>
                {
                    rows.push(start..space);
                    start = space + 1;
                    used = width(&line[start..end]);
                }
                _ => {
                    rows.push(start..end);
                    (start, used) = (at, 0);
                }
            }
            seam = None;
        }
        if unit == " " {
            seam = Some(at);
        }
        end = next;
        used += size;
    }
    rows.push(start..end);
    rows
}

/// Sanitize and word-wrap to `columns` cells; blank lines are kept.
pub fn wrap(text: &str, columns: usize) -> Vec<String> {
    let clean = safe(text);
    let mut rows = Vec::new();
    for line in clean.split('\n') {
        for range in wrap_line(line, columns) {
            let row = &line[range];
            // A wide cluster in a one-cell column is replaced, never split.
            rows.push(if width(row) > columns.max(1) {
                "?".to_string()
            } else {
                row.to_string()
            });
        }
    }
    rows
}

/// Fit `text` into `columns` cells, ending with `ellipsis` when cut.
pub fn clip(text: &str, columns: usize, ellipsis: &str) -> String {
    if width(text) <= columns {
        return text.to_string();
    }
    let mark = width(ellipsis);
    if mark > columns {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for unit in clusters(text) {
        let size = cluster_width(unit);
        if used + size + mark > columns {
            break;
        }
        out.push_str(unit);
        used += size;
    }
    out.push_str(ellipsis);
    out
}

/// Right-pad with spaces to exactly `columns` cells (no-op when wider).
pub fn pad(text: &str, columns: usize) -> String {
    let mut out = text.to_string();
    out.push_str(&" ".repeat(columns.saturating_sub(width(text))));
    out
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
