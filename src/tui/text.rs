// Approximate terminal cell widths for common combining marks, CJK and emoji.
// The original message remains untouched in the conversation archive.
pub fn width(character: char) -> usize {
    match character as u32 {
        0x0300..=0x036f
        | 0x1ab0..=0x1aff
        | 0x1dc0..=0x1dff
        | 0x200d
        | 0x20d0..=0x20ff
        | 0xfe00..=0xfe0f
        | 0xfe20..=0xfe2f
        | 0x1f3fb..=0x1f3ff => 0,
        0x1100..=0x115f
        | 0x2329..=0x232a
        | 0x2e80..=0xa4cf
        | 0xac00..=0xd7a3
        | 0xf900..=0xfaff
        | 0xfe10..=0xfe6f
        | 0xff01..=0xff60
        | 0x1f1e6..=0x1f1ff
        | 0x1f300..=0x1faff
        | 0x20000..=0x3ffff => 2,
        _ => 1,
    }
}

pub struct Glyph<'a> {
    pub text: &'a str,
    pub cells: usize,
}

pub struct Glyphs<'a> {
    source: &'a str,
    offset: usize,
}

pub fn glyphs(source: &str) -> Glyphs<'_> {
    Glyphs { source, offset: 0 }
}

pub(super) fn extend(character: char) -> bool {
    width(character) == 0 && character != '\u{200d}'
}

pub(super) fn emoji_base(character: char) -> bool {
    matches!(
        character as u32,
        0x00a9 | 0x00ae | 0x2122 | 0x2300..=0x23ff | 0x2600..=0x27bf | 0x1f000..=0x1faff
    )
}

impl<'a> Iterator for Glyphs<'a> {
    type Item = Glyph<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let tail = self.source.get(self.offset..)?;
        let first = tail.chars().next()?;
        let start = self.offset;
        self.offset += first.len_utf8();
        if first == '\n' {
            return Some(Glyph {
                text: &self.source[start..self.offset],
                cells: width(first),
            });
        }
        let mut cells = width(first);
        let mut emoji = emoji_base(first);
        let mut regional = matches!(first as u32, 0x1f1e6..=0x1f1ff);
        while let Some(next) = self.source[self.offset..].chars().next() {
            if extend(next) {
                self.offset += next.len_utf8();
                match next {
                    '\u{fe0e}' if emoji => cells = 1,
                    '\u{fe0f}' if emoji || matches!(first, '0'..='9' | '#' | '*') => {
                        cells = 2;
                        emoji = true;
                    }
                    '\u{20e3}' => cells = 2,
                    _ => {}
                }
            } else if next == '\u{200d}' && emoji {
                let after = self.source[self.offset + next.len_utf8()..].chars().next();
                if !after.is_some_and(emoji_base) {
                    break;
                }
                self.offset += next.len_utf8() + after.unwrap().len_utf8();
                cells = 2;
            } else if regional && matches!(next as u32, 0x1f1e6..=0x1f1ff) {
                self.offset += next.len_utf8();
                cells = 2;
                regional = false;
            } else {
                break;
            }
        }
        Some(Glyph {
            text: &self.source[start..self.offset],
            cells,
        })
    }
}
pub fn clean(text: &str) -> String {
    text.replace("\r\n", "\n")
        .chars()
        .flat_map(|character| match character {
            '\n' => "\n".chars().collect::<Vec<_>>(),
            '\t' => "    ".chars().collect(),
            '\r' => vec![],
            '\x1b' => "[ESC]".chars().collect(),
            character if character.is_control() || ('\u{7f}'..='\u{9f}').contains(&character) => {
                vec!['�']
            }
            character => vec![character],
        })
        .collect()
}
pub fn clip(text: &str, columns: usize) -> String {
    let mut result = String::new();
    let mut used = 0;
    let cleaned = clean(text);
    for glyph in glyphs(&cleaned) {
        if glyph.text == "\n" || used + glyph.cells > columns {
            break;
        }
        result.push_str(glyph.text);
        used += glyph.cells;
    }
    result
}
pub fn cells(value: &str) -> usize {
    glyphs(value).map(|glyph| glyph.cells).sum()
}
pub fn ellipsize(value: &str, columns: usize) -> String {
    if cells(value) <= columns {
        return value.into();
    }
    if columns == 0 {
        return String::new();
    }
    let mut prefix = String::new();
    let mut used = 0;
    for glyph in glyphs(value) {
        if used + glyph.cells >= columns {
            break;
        }
        prefix.push_str(glyph.text);
        used += glyph.cells;
    }
    format!("{prefix}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wraps_unicode_and_escapes_terminal_controls() {
        assert_eq!(clip("città🙂z", 7), "città🙂");
        assert_eq!(clean("\x1b[2J\r\nnew\tline"), "[ESC][2J\nnew    line");
        assert_eq!(clip("e\u{301}x", 2), "e\u{301}x");
        assert_eq!(clip("ab", 1), "a");
        assert_eq!(clip("ab", 0), "");
    }

    #[test]
    fn common_display_sequences_are_counted_and_clipped_as_whole_glyphs() {
        let cases = [
            ("e\u{301}", 1),
            ("\u{754c}", 2),
            ("\u{2764}\u{fe0f}", 2),
            ("1\u{fe0f}\u{20e3}", 2),
            ("\u{1f1ee}\u{1f1f9}", 2),
            ("\u{1f469}\u{1f3fd}\u{200d}\u{1f4bb}", 2),
        ];
        for (source, expected) in cases {
            assert_eq!(cells(source), expected, "{source}");
            assert_eq!(glyphs(source).count(), 1, "{source}");
            assert_eq!(
                clip(&format!("{source}x"), 1),
                if expected == 1 { source } else { "" }
            );
            assert_eq!(clip(&format!("{source}x"), expected), source);
        }
        let joined = "\u{1f469}\u{200d}\u{1f4bb}";
        assert_eq!(ellipsize(&format!("A{joined}B"), 3), "A…");
        assert_eq!(ellipsize(&format!("A{joined}B"), 4), format!("A{joined}B"));
        assert_eq!(
            glyphs("x\n\u{301}z")
                .map(|glyph| glyph.text)
                .collect::<Vec<_>>(),
            ["x", "\n", "\u{301}", "z"]
        );
    }
}
