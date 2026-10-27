use crate::{
    file_change::{Entry, Kind},
    tui::{
        line::Line,
        text,
        theme::{BAD, DIFF_ADDED_STRONG, DIFF_REMOVED_STRONG, GOOD},
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

/// Display tokens: in markup a whole tag `<…>`, comment or entity `&…;`;
/// elsewhere a word, a run of whitespace or one other glyph.
fn tokens(source: &str, markup: bool) -> Vec<&str> {
    let mut starts: Vec<usize> = text::glyphs(source)
        .scan(0, |at, glyph| {
            let start = *at;
            *at += glyph.text.len();
            Some(start)
        })
        .collect();
    starts.push(source.len());
    let class = |index: usize| {
        let character = source[starts[index]..].chars().next().unwrap();
        if character.is_whitespace() {
            1
        } else if character.is_alphanumeric() || character == '_' {
            2
        } else {
            0
        }
    };
    let mut tokens = Vec::new();
    let mut index = 0;
    while index + 1 < starts.len() {
        let next = match markup
            .then(|| markup_token(&source[starts[index]..]))
            .flatten()
        {
            Some(length) => starts.partition_point(|&start| start < starts[index] + length),
            None => {
                let kind = class(index);
                let mut next = index + 1;
                while kind != 0 && next + 1 < starts.len() && class(next) == kind {
                    next += 1;
                }
                next
            }
        };
        tokens.push(&source[starts[index]..starts[next]]);
        index = next;
    }
    tokens
}

fn markup_token(tail: &str) -> Option<usize> {
    if tail.starts_with("<!--") {
        return Some(tail.find("-->").map_or(tail.len(), |end| end + 3));
    }
    if let Some(inner) = tail.strip_prefix('<') {
        return Some(inner.find(['<', '>']).map_or(tail.len(), |end| {
            end + 1 + usize::from(inner[end..].starts_with('>'))
        }));
    }
    let entity = tail.strip_prefix('&')?;
    let name = entity.strip_prefix('#').unwrap_or(entity);
    let word = name
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        .count();
    (word > 0 && name[word..].starts_with(';')).then(|| tail.len() - name.len() + word + 1)
}

/// The changed range of `source` against `other`, keeping whole tokens: the
/// common prefix and suffix are counted in tokens, so the range only grows
/// past a character comparison and both partners keep one shared prefix.
pub(super) fn changed(source: &str, other: &str, markup: bool) -> Range<usize> {
    let ours = tokens(source, markup);
    let theirs = tokens(other, markup);
    let prefix = ours.iter().zip(&theirs).take_while(|(a, b)| a == b).count();
    let suffix = ours[prefix..]
        .iter()
        .rev()
        .zip(theirs[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let start = ours[..prefix].iter().map(|token| token.len()).sum();
    let tail: usize = ours[ours.len() - suffix..]
        .iter()
        .map(|token| token.len())
        .sum();
    start..source.len() - tail
}

/// Keep syntax outside the changed fragment; the fragment owns its surface.
pub(super) fn highlight(code: Line, range: Option<Range<usize>>, kind: Kind) -> Line {
    let Some(range) = range.filter(|range| !range.is_empty()) else {
        return code;
    };
    let (style, background) = if kind == Kind::Removed {
        (BAD.bold(), DIFF_REMOVED_STRONG)
    } else {
        (GOOD.bold(), DIFF_ADDED_STRONG)
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
