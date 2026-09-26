//! One grapheme segmenter and one width model for every consumer: wrapping,
//! clipping, padding, frame heights and the editor. Compact hand-written
//! tables, not a full Unicode database; every rule here is covered by tests.
use std::cmp::Ordering;

const ZWJ: char = '\u{200d}';
const VS15: char = '\u{fe0e}';
const VS16: char = '\u{fe0f}';

/// Byte offsets of grapheme boundaries, always including 0 and `text.len()`.
pub fn boundaries(text: &str) -> Vec<usize> {
    let mut result = vec![0];
    let mut previous = None;
    let mut regional_run = 0usize;
    for (index, current) in text.char_indices() {
        if let Some(previous) = previous
            && breaks(previous, current, regional_run)
        {
            result.push(index);
        }
        regional_run = if regional(current) {
            regional_run + 1
        } else {
            0
        };
        previous = Some(current);
    }
    if !text.is_empty() {
        result.push(text.len());
    }
    result
}

/// The grapheme clusters of `text`, in order.
pub fn clusters(text: &str) -> impl Iterator<Item = &str> {
    let stops = boundaries(text);
    (1..stops.len()).map(move |index| &text[stops[index - 1]..stops[index]])
}

/// Terminal cells occupied by `text`.
pub fn width(text: &str) -> usize {
    clusters(text).map(cluster_width).sum()
}

/// Cells for one cluster: presentation selectors win, flag pairs are wide,
/// otherwise the base character decides.
pub fn cluster_width(cluster: &str) -> usize {
    let mut chars = cluster.chars();
    let Some(base) = chars.next() else {
        return 0;
    };
    if cluster.contains(VS16) && (pictograph(base) || matches!(base, '0'..='9' | '#' | '*')) {
        return 2;
    }
    if cluster.contains(VS15) {
        return 1;
    }
    if regional(base) {
        return if chars.any(regional) { 2 } else { 1 };
    }
    char_width(base)
}

fn breaks(previous: char, current: char, regional_run: usize) -> bool {
    if previous == '\r' && current == '\n' {
        return false;
    }
    if previous.is_control() || current.is_control() {
        return true;
    }
    if extend(current) || current == ZWJ || conjoining_hangul(previous, current) {
        return false;
    }
    if prepend(previous) || previous == ZWJ && pictograph(current) {
        return false;
    }
    if virama(previous) && current.is_alphabetic() {
        return false;
    }
    if regional(previous) && regional(current) {
        // Flags pair up left to right: 🇮🇹🇫🇷 is two clusters, not one or four.
        return regional_run.is_multiple_of(2);
    }
    true
}

fn char_width(ch: char) -> usize {
    if zero_width(ch) {
        0
    } else if wide(ch) {
        2
    } else {
        1
    }
}

fn zero_width(ch: char) -> bool {
    ch.is_control()
        || extend(ch)
        || matches!(
            ch,
            ZWJ | '\u{200b}' | '\u{200e}' | '\u{200f}' | '\u{2060}'..='\u{2064}' | '\u{feff}'
        )
        || matches!(hangul(ch), Hangul::V | Hangul::T)
}

fn wide(ch: char) -> bool {
    let code = ch as u32;
    WIDE.binary_search_by(|&(low, high)| {
        if high < code {
            Ordering::Less
        } else if low > code {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    })
    .is_ok()
}

fn regional(ch: char) -> bool {
    ('\u{1f1e6}'..='\u{1f1ff}').contains(&ch)
}

fn pictograph(ch: char) -> bool {
    matches!(ch,
        '\u{a9}' | '\u{ae}' | '\u{203c}' | '\u{2049}' | '\u{2122}' | '\u{2139}' |
        '\u{2190}'..='\u{2bff}' | '\u{3030}' | '\u{303d}' | '\u{3297}' | '\u{3299}' |
        '\u{1f000}'..='\u{1faff}')
}

fn extend(ch: char) -> bool {
    matches!(ch,
        '\u{0300}'..='\u{036f}' | '\u{0483}'..='\u{0489}' | '\u{0591}'..='\u{05bd}' |
        '\u{05bf}' | '\u{05c1}'..='\u{05c2}' | '\u{05c4}'..='\u{05c5}' | '\u{05c7}' |
        '\u{0610}'..='\u{061a}' | '\u{064b}'..='\u{065f}' | '\u{0670}' |
        '\u{06d6}'..='\u{06dc}' | '\u{06df}'..='\u{06e4}' | '\u{06e7}'..='\u{06e8}' |
        '\u{06ea}'..='\u{06ed}' | '\u{0900}'..='\u{0903}' | '\u{093a}'..='\u{094f}' |
        '\u{0951}'..='\u{0957}' | '\u{0962}'..='\u{0963}' | '\u{0e31}' |
        '\u{0e34}'..='\u{0e3a}' | '\u{0e47}'..='\u{0e4e}' | '\u{1ab0}'..='\u{1aff}' |
        '\u{1dc0}'..='\u{1dff}' | '\u{200c}' | '\u{20d0}'..='\u{20ff}' |
        '\u{302a}'..='\u{302f}' | '\u{3099}'..='\u{309a}' | '\u{fe00}'..='\u{fe0f}' |
        '\u{fe20}'..='\u{fe2f}' | '\u{1f3fb}'..='\u{1f3ff}' | '\u{e0020}'..='\u{e007f}' |
        '\u{e0100}'..='\u{e01ef}')
}

fn prepend(ch: char) -> bool {
    matches!(
        ch,
        '\u{0600}'..='\u{0605}' | '\u{06dd}' | '\u{070f}' | '\u{08e2}'
    )
}

fn virama(ch: char) -> bool {
    matches!(
        ch,
        '\u{094d}'
            | '\u{09cd}'
            | '\u{0a4d}'
            | '\u{0acd}'
            | '\u{0b4d}'
            | '\u{0bcd}'
            | '\u{0c4d}'
            | '\u{0ccd}'
            | '\u{0d4d}'
    )
}

#[derive(Clone, Copy)]
enum Hangul {
    L,
    V,
    T,
    Lv,
    Lvt,
    Other,
}

fn hangul(ch: char) -> Hangul {
    match ch {
        '\u{1100}'..='\u{115f}' | '\u{a960}'..='\u{a97c}' => Hangul::L,
        '\u{1160}'..='\u{11a7}' | '\u{d7b0}'..='\u{d7c6}' => Hangul::V,
        '\u{11a8}'..='\u{11ff}' | '\u{d7cb}'..='\u{d7fb}' => Hangul::T,
        '\u{ac00}'..='\u{d7a3}' if (ch as u32 - 0xac00).is_multiple_of(28) => Hangul::Lv,
        '\u{ac00}'..='\u{d7a3}' => Hangul::Lvt,
        _ => Hangul::Other,
    }
}

fn conjoining_hangul(previous: char, current: char) -> bool {
    use Hangul::{L, Lv, Lvt, T, V};
    matches!(
        (hangul(previous), hangul(current)),
        (L, L | V | Lv | Lvt) | (Lv | V, V | T) | (Lvt | T, T)
    )
}

/// East Asian Wide/Fullwidth blocks plus default emoji presentation.
/// Sorted, non-overlapping, inclusive; searched with `binary_search_by`.
const WIDE: &[(u32, u32)] = &[
    (0x1100, 0x115f),
    (0x231a, 0x231b),
    (0x2329, 0x232a),
    (0x23e9, 0x23ec),
    (0x23f0, 0x23f0),
    (0x23f3, 0x23f3),
    (0x25fd, 0x25fe),
    (0x2614, 0x2615),
    (0x2648, 0x2653),
    (0x267f, 0x267f),
    (0x2693, 0x2693),
    (0x26a1, 0x26a1),
    (0x26aa, 0x26ab),
    (0x26bd, 0x26be),
    (0x26c4, 0x26c5),
    (0x26ce, 0x26ce),
    (0x26d4, 0x26d4),
    (0x26ea, 0x26ea),
    (0x26f2, 0x26f3),
    (0x26f5, 0x26f5),
    (0x26fa, 0x26fa),
    (0x26fd, 0x26fd),
    (0x2705, 0x2705),
    (0x270a, 0x270b),
    (0x2728, 0x2728),
    (0x274c, 0x274c),
    (0x274e, 0x274e),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2795, 0x2797),
    (0x27b0, 0x27b0),
    (0x27bf, 0x27bf),
    (0x2b1b, 0x2b1c),
    (0x2b50, 0x2b50),
    (0x2b55, 0x2b55),
    (0x2e80, 0x303e),
    (0x3041, 0x33ff),
    (0x3400, 0x4dbf),
    (0x4e00, 0x9fff),
    (0xa000, 0xa4cf),
    (0xa960, 0xa97f),
    (0xac00, 0xd7a3),
    (0xf900, 0xfaff),
    (0xfe10, 0xfe19),
    (0xfe30, 0xfe6f),
    (0xff00, 0xff60),
    (0xffe0, 0xffe6),
    (0x16fe0, 0x18aff),
    (0x1b000, 0x1b2ff),
    (0x1f004, 0x1f004),
    (0x1f0cf, 0x1f0cf),
    (0x1f18e, 0x1f18e),
    (0x1f191, 0x1f19a),
    (0x1f200, 0x1f202),
    (0x1f210, 0x1f23b),
    (0x1f240, 0x1f248),
    (0x1f250, 0x1f251),
    (0x1f260, 0x1f265),
    (0x1f300, 0x1f320),
    (0x1f32d, 0x1f335),
    (0x1f337, 0x1f37c),
    (0x1f37e, 0x1f393),
    (0x1f3a0, 0x1f3ca),
    (0x1f3cf, 0x1f3d3),
    (0x1f3e0, 0x1f3f0),
    (0x1f3f4, 0x1f3f4),
    (0x1f3f8, 0x1f43e),
    (0x1f440, 0x1f440),
    (0x1f442, 0x1f4fc),
    (0x1f4ff, 0x1f53d),
    (0x1f54b, 0x1f54e),
    (0x1f550, 0x1f567),
    (0x1f57a, 0x1f57a),
    (0x1f595, 0x1f596),
    (0x1f5a4, 0x1f5a4),
    (0x1f5fb, 0x1f64f),
    (0x1f680, 0x1f6c5),
    (0x1f6cc, 0x1f6cc),
    (0x1f6d0, 0x1f6d2),
    (0x1f6d5, 0x1f6d7),
    (0x1f6dc, 0x1f6df),
    (0x1f6eb, 0x1f6ec),
    (0x1f6f4, 0x1f6fc),
    (0x1f7e0, 0x1f7eb),
    (0x1f7f0, 0x1f7f0),
    (0x1f90c, 0x1f93a),
    (0x1f93c, 0x1f945),
    (0x1f947, 0x1f9ff),
    (0x1fa70, 0x1faff),
    (0x20000, 0x2fffd),
    (0x30000, 0x3fffd),
];

#[cfg(test)]
#[path = "unicode_tests.rs"]
mod tests;
