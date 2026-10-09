use super::{
    text,
    theme::{CURSOR, INLINE_CODE},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: &'static str,
    pub background: Option<&'static str>,
}

#[derive(Clone, Debug, Default)]
pub struct Line {
    pub spans: Vec<Span>,
    pub background: Option<&'static str>,
    pub origin: Option<Origin>,
}

impl PartialEq for Line {
    fn eq(&self, other: &Self) -> bool {
        self.spans == other.spans && self.background == other.background
    }
}
impl Eq for Line {}

// A source line and a character in its rendered text survive word wrapping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Origin {
    pub line: usize,
    pub character: usize,
}

type StyledChar = (char, &'static str, Option<&'static str>);

struct StyledGlyph {
    start: usize,
    end: usize,
    cells: usize,
    newline: bool,
    whitespace: bool,
    breakable: bool,
}

struct StyledLine {
    characters: Vec<StyledChar>,
    glyphs: Vec<StyledGlyph>,
}

pub(in crate::tui) struct Wrapped {
    styled: StyledLine,
    start: usize,
    columns: usize,
    words: bool,
    background: Option<&'static str>,
    empty_pending: bool,
    origin: Option<Origin>,
}

impl Line {
    pub fn at_source(mut self, line: usize) -> Self {
        self.origin = Some(Origin { line, character: 0 });
        self
    }
    pub fn new(value: &str, style: &'static str) -> Self {
        let mut line = Self::default();
        line.push(value, style);
        line
    }
    pub fn push(&mut self, value: &str, style: &'static str) {
        self.push_span(value, style, None);
    }
    fn push_span(&mut self, value: &str, style: &'static str, background: Option<&'static str>) {
        if value.is_empty() {
            return;
        }
        if let Some(last) = self.spans.last_mut()
            && last.style == style
            && last.background == background
        {
            last.text.push_str(value);
        } else {
            self.spans.push(Span {
                text: value.into(),
                style,
                background,
            });
        }
    }
    pub fn plain(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }
    pub fn indent(mut self, prefix: &str, style: &'static str) -> Self {
        self.spans.insert(
            0,
            Span {
                text: prefix.into(),
                style,
                background: None,
            },
        );
        self
    }
    pub fn gutter(mut self, prefix: &str, style: &'static str) -> Self {
        // A tree guide inherits the terminal surface, even beside shaded diffs.
        self.spans.insert(
            0,
            Span {
                text: prefix.into(),
                style,
                background: Some("49"),
            },
        );
        self
    }
    pub fn on(mut self, background: &'static str) -> Self {
        self.background = Some(background);
        self
    }
    fn glyphs(&self) -> StyledLine {
        let characters: Vec<_> = self
            .spans
            .iter()
            .flat_map(|span| {
                span.text
                    .chars()
                    .map(|character| (character, span.style, span.background))
            })
            .collect();
        let plain = self.plain();
        let mut start = 0;
        let glyphs = text::glyphs(&plain)
            .map(|glyph| {
                let end = start + glyph.text.chars().count();
                let part = StyledGlyph {
                    start,
                    end,
                    cells: glyph.cells,
                    newline: glyph.text == "\n",
                    whitespace: glyph.text.chars().next().is_some_and(char::is_whitespace),
                    breakable: glyph.text.chars().next().is_some_and(char::is_whitespace)
                        && characters[start].1 != INLINE_CODE,
                };
                start = end;
                part
            })
            .collect();
        StyledLine { characters, glyphs }
    }
    fn push_glyph(
        &mut self,
        characters: &[StyledChar],
        glyph: &StyledGlyph,
        style: Option<&'static str>,
    ) {
        for &(character, original, background) in &characters[glyph.start..glyph.end] {
            let mut utf8 = [0; 4];
            self.push_span(
                character.encode_utf8(&mut utf8),
                style.unwrap_or(original),
                background,
            );
        }
    }
    pub fn shortened(&self, columns: usize) -> Self {
        if self.spans.iter().any(|span| span.text.contains('\n')) {
            let mut single = self.clone();
            for span in &mut single.spans {
                span.text = span.text.replace('\n', " ↵ ");
            }
            return single.shortened(columns);
        }
        if text::cells(&self.plain()) <= columns {
            return self.clone();
        }
        let styled = self.glyphs();
        let mut line = Self {
            background: self.background,
            origin: self.origin,
            ..Self::default()
        };
        if columns == 0 {
            return line;
        }
        let mut cells = 0;
        let mut style = "0";
        let mut background = None;
        for glyph in &styled.glyphs {
            if cells + glyph.cells > columns - 1 {
                break;
            }
            cells += glyph.cells;
            if let Some((_, selected_style, selected_background)) =
                styled.characters[glyph.start..glyph.end].last()
            {
                style = *selected_style;
                background = *selected_background;
            }
            line.push_glyph(&styled.characters, glyph, None);
        }
        line.push_span("…", style, background);
        line
    }
    pub fn caret(&self, column: usize, style: &'static str) -> Self {
        let mut line = Self {
            background: self.background,
            ..Self::default()
        };
        let mut cells = 0;
        let mut covered = false;
        let styled = self.glyphs();
        for glyph in &styled.glyphs {
            let selected = glyph.cells > 0 && column >= cells && column < cells + glyph.cells;
            if selected {
                covered = true;
            }
            line.push_glyph(&styled.characters, glyph, selected.then_some(style));
            cells += glyph.cells;
        }
        if !covered && cells <= column {
            line.push(&" ".repeat(column - cells), "0");
            line.push(" ", style);
        }
        line
    }
    pub fn paint(&self, columns: usize) -> String {
        let mut output = String::from("\x1b[0m");
        let mut cells = 0;
        let mut painted = Self::default();
        let styled = self.glyphs();
        for glyph in &styled.glyphs {
            if cells + glyph.cells > columns {
                break;
            }
            cells += glyph.cells;
            painted.push_glyph(&styled.characters, glyph, None);
        }
        for span in &painted.spans {
            output.push_str(&format!("\x1b[0m\x1b[{}m", span.style));
            // The software caret carries its own background, even inside a panel.
            if let Some(background) = span.background.or(self.background)
                && span.style != CURSOR
            {
                output.push_str(&format!("\x1b[{background}m"));
            }
            output.push_str(&span.text);
        }
        if let Some(style) = self.background {
            output.push_str(&format!(
                "\x1b[0m\x1b[{style}m{}",
                " ".repeat(columns - cells)
            ));
        }
        output.push_str("\x1b[0m");
        output
    }
    pub fn wrap(&self, columns: usize, words: bool) -> Vec<Self> {
        Wrapped::new(self, columns, words).collect()
    }
    pub(in crate::tui) fn into_wrapped(self, columns: usize, words: bool) -> Wrapped {
        Wrapped::new(&self, columns, words)
    }
}

impl Wrapped {
    fn new(line: &Line, columns: usize, words: bool) -> Self {
        let styled = line.glyphs();
        let empty_pending =
            styled.glyphs.is_empty() || styled.glyphs.last().is_some_and(|glyph| glyph.newline);
        Self {
            styled,
            start: 0,
            columns: columns.max(2),
            words,
            background: line.background,
            empty_pending,
            origin: line.origin,
        }
    }
}

impl Iterator for Wrapped {
    type Item = Line;

    fn next(&mut self) -> Option<Self::Item> {
        let glyphs = &self.styled.glyphs;
        if self.start >= glyphs.len() {
            if !self.empty_pending {
                return None;
            }
            self.empty_pending = false;
            return Some(Line {
                background: self.background,
                origin: self.origin,
                ..Line::default()
            });
        }
        let mut end = self.start;
        let mut cells = 0;
        while end < glyphs.len() && !glyphs[end].newline {
            if cells + glyphs[end].cells > self.columns {
                break;
            }
            cells += glyphs[end].cells;
            end += 1;
        }
        let hard_break = end < glyphs.len() && glyphs[end].newline;
        let mut following = end + usize::from(hard_break);
        if self.words
            && !hard_break
            && end < glyphs.len()
            && let Some(space) = (self.start..=end)
                .rev()
                .find(|&index| glyphs[index].breakable)
            && glyphs[self.start..space]
                .iter()
                .any(|glyph| !glyph.whitespace)
        {
            end = space;
            following = space + 1;
            while following < glyphs.len()
                && glyphs[following].breakable
                && self.styled.characters[glyphs[following].start].0 == ' '
            {
                following += 1;
            }
        }
        let mut line = Line {
            background: self.background,
            origin: self.origin.map(|origin| Origin {
                character: origin.character + glyphs[self.start].start,
                ..origin
            }),
            ..Line::default()
        };
        for glyph in &glyphs[self.start..end] {
            line.push_glyph(&self.styled.characters, glyph, None);
        }
        self.start = following;
        Some(line)
    }
}

#[cfg(test)]
mod tests;
