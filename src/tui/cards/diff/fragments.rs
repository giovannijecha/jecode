use crate::{
    file_change::{Entry, Kind},
    tui::{
        line::Line,
        text,
        theme::{BAD, DIFF_ADDED_SURFACE, DIFF_REMOVED_SURFACE, GOOD},
    },
};
use std::ops::Range;

/// Pair neighboring removal/addition runs by position, without realigning lines.
pub(super) fn partners(entries: &[Entry]) -> Vec<Option<usize>> {
    let mut partners = vec![None; entries.len()];
    let mut at = 0;
    while at < entries.len() {
        if entries[at].kind != Kind::Removed {
            at += 1;
            continue;
        }
        let removed = at;
        while at < entries.len() && entries[at].kind == Kind::Removed {
            at += 1;
        }
        let added = at;
        while at < entries.len() && entries[at].kind == Kind::Added {
            at += 1;
        }
        for offset in 0..(added - removed).min(at - added) {
            partners[removed + offset] = Some(added + offset);
            partners[added + offset] = Some(removed + offset);
        }
    }
    partners
}

/// A contiguous changed range, keeping combining sequences and emoji intact.
fn changed(source: &str, other: &str) -> Range<usize> {
    let source_glyphs: Vec<_> = text::glyphs(source).map(|glyph| glyph.text).collect();
    let other_glyphs: Vec<_> = text::glyphs(other).map(|glyph| glyph.text).collect();
    let prefix = source_glyphs
        .iter()
        .zip(&other_glyphs)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = source_glyphs[prefix..]
        .iter()
        .rev()
        .zip(other_glyphs[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let start = source_glyphs[..prefix]
        .iter()
        .map(|glyph| glyph.len())
        .sum();
    let tail: usize = source_glyphs[source_glyphs.len() - suffix..]
        .iter()
        .map(|glyph| glyph.len())
        .sum();
    start..source.len() - tail
}

/// Keep syntax outside the changed fragment; the fragment owns its surface.
pub(super) fn highlight(code: Line, source: &str, other: Option<&str>, kind: Kind) -> Line {
    let range = other.map_or_else(
        || 0..source.len(),
        |other| changed(source, &text::clean(other)),
    );
    if range.is_empty() {
        return code;
    }
    let (style, background) = if kind == Kind::Removed {
        (BAD.bold(), DIFF_REMOVED_SURFACE)
    } else {
        (GOOD.bold(), DIFF_ADDED_SURFACE)
    };
    let mut result = Line::default();
    let mut at = 0;
    for span in code.spans {
        let end = at + span.text.len();
        let start = range.start.clamp(at, end) - at;
        let stop = range.end.clamp(at, end) - at;
        for (part, changed) in [
            (&span.text[..start], false),
            (&span.text[start..stop], true),
            (&span.text[stop..], false),
        ] {
            if changed {
                result.push_on(part, style, background);
            } else if let Some(original) = span.background {
                result.push_on(part, span.style, original);
            } else {
                result.push(part, span.style);
            }
        }
        at = end;
    }
    result
}
