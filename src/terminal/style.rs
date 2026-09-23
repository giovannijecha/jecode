//! Owned presentation tokens. Incoming text never supplies ANSI sequences.
use super::text;
use std::ops::Range;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    Text,
    Muted,
    Accent,
    User,
    Code,
    Added,
    Removed,
    Heading,
    Error,
    Success,
    Keyword,
    String,
    Number,
    Cursor,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Row {
    pub text: String,
    pub tone: Tone,
    pub spans: Vec<(Range<usize>, Tone)>,
    /// Transient input/status rows, never part of the emitted conversation.
    pub transient: bool,
}
impl Row {
    pub fn new(text: impl Into<String>, tone: Tone) -> Self {
        Self {
            text: text.into(),
            tone,
            spans: Vec::new(),
            transient: false,
        }
    }
    pub fn blank() -> Self {
        Self::new("", Tone::Text)
    }
    pub fn paint(&self, color: bool) -> String {
        let mut out = String::new();
        if color {
            out.push_str(self.tone.ansi());
        }
        let mut offset = 0;
        for (range, tone) in &self.spans {
            out.push_str(&self.text[offset..range.start]);
            if color {
                out.push_str(tone.ansi());
            }
            if !color && *tone == Tone::Cursor {
                out.push('|');
            } else {
                out.push_str(&self.text[range.clone()]);
            }
            if color {
                out.push_str(self.tone.ansi());
            }
            offset = range.end;
        }
        out.push_str(&self.text[offset..]);
        if color {
            out.push_str("\x1b[0m");
        }
        out
    }
}
impl Tone {
    fn ansi(self) -> &'static str {
        match self {
            Self::Text => "\x1b[0m",
            Self::Muted => "\x1b[0;38;2;154;164;178m",
            Self::Heading => "\x1b[0;1;38;2;220;228;240m",
            Self::Accent => "\x1b[0;38;2;122;162;247m",
            Self::User => "\x1b[0;48;2;40;46;57;38;2;226;232;240m",
            Self::Code => "\x1b[0;48;2;32;37;44;38;2;201;209;217m",
            Self::Added => "\x1b[0;48;2;30;46;35;38;2;158;206;106m",
            Self::Removed => "\x1b[0;48;2;49;32;38;38;2;239;139;139m",
            Self::Error => "\x1b[0;38;2;239;139;139m",
            Self::Success => "\x1b[0;38;2;158;206;106m",
            Self::Keyword => "\x1b[38;2;187;154;247m",
            Self::String => "\x1b[38;2;158;206;106m",
            Self::Number => "\x1b[38;2;224;175;104m",
            Self::Cursor => "\x1b[0;48;2;192;202;245;38;2;26;27;38m",
        }
    }
}
pub fn lines(value: &str, width: usize, tone: Tone) -> Vec<Row> {
    text::wrap(value, width)
        .into_iter()
        .map(|line| Row::new(line, tone))
        .collect()
}
pub fn pad(mut row: Row, width: usize) -> Row {
    row.text
        .push_str(&" ".repeat(width.saturating_sub(text::width(&row.text))));
    row
}
