//! Owned presentation tokens. Incoming text never supplies escape sequences;
//! each `Tone` resolves to SGR for the detected color depth. Truecolor values
//! are jecode's identity and must not drift; 256/16 are nearest fallbacks.
use super::caps::ColorDepth;
use super::text;
use std::fmt::Write;
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
    Warning,
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
    /// Sorted, non-overlapping byte ranges drawn in another tone.
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

    /// Append `text`, styled as a span unless it matches the row tone.
    pub fn push(&mut self, text: &str, tone: Tone) -> &mut Self {
        let start = self.text.len();
        self.text.push_str(text);
        if tone != self.tone && !text.is_empty() {
            self.spans.push((start..self.text.len(), tone));
        }
        self
    }

    /// A foreground-only span on a row with a background keeps that
    /// background (an accent inside the user panel, syntax inside code).
    /// Any other span replaces the row style until it ends.
    pub fn paint(&self, depth: ColorDepth) -> String {
        let base = self.tone.sgr(depth);
        let layered = self.tone.spec().bg.is_some();
        let mut out = String::with_capacity(self.text.len() + 24);
        let mut styled = !base.is_empty();
        out.push_str(&base);
        let mut offset = 0;
        for (range, tone) in &self.spans {
            out.push_str(&self.text[offset..range.start]);
            let spec = tone.spec();
            let overlay = layered && spec.fg.is_some() && spec.bg.is_none();
            let sgr = tone.codes(depth, !overlay);
            out.push_str(&sgr);
            out.push_str(&self.text[range.clone()]);
            if !sgr.is_empty() {
                styled = true;
                out.push_str(if base.is_empty() { RESET } else { &base });
            }
            offset = range.end;
        }
        out.push_str(&self.text[offset..]);
        if styled {
            out.push_str(RESET);
        }
        out
    }
}

const RESET: &str = "\x1b[0m";

#[derive(Clone, Copy)]
struct Ink {
    rgb: (u8, u8, u8),
    xterm: u8,
    /// SGR foreground code for 16 colors; 0 means "leave default".
    ansi: u8,
}

const fn ink(r: u8, g: u8, b: u8, xterm: u8, ansi: u8) -> Ink {
    Ink {
        rgb: (r, g, b),
        xterm,
        ansi,
    }
}

struct Spec {
    fg: Option<Ink>,
    bg: Option<Ink>,
    bold: bool,
}

impl Tone {
    fn spec(self) -> Spec {
        let fg = |ink| (Some(ink), None, false);
        let (fg, bg, bold) = match self {
            Self::Text | Self::Cursor => (None, None, false),
            Self::Muted => fg(ink(154, 164, 178, 248, 90)),
            Self::Heading => (Some(ink(220, 228, 240, 255, 97)), None, true),
            Self::Accent => fg(ink(122, 162, 247, 111, 94)),
            Self::User => (
                Some(ink(226, 232, 240, 254, 0)),
                Some(ink(40, 46, 57, 236, 0)),
                false,
            ),
            Self::Code => (
                Some(ink(201, 209, 217, 252, 0)),
                Some(ink(32, 37, 44, 235, 0)),
                false,
            ),
            Self::Added => (
                Some(ink(158, 206, 106, 149, 32)),
                Some(ink(30, 46, 35, 22, 0)),
                false,
            ),
            Self::Removed => (
                Some(ink(239, 139, 139, 210, 31)),
                Some(ink(49, 32, 38, 52, 0)),
                false,
            ),
            Self::Error => fg(ink(239, 139, 139, 210, 91)),
            Self::Warning => fg(ink(224, 175, 104, 179, 33)),
            Self::Success | Self::String => fg(ink(158, 206, 106, 149, 32)),
            Self::Keyword => fg(ink(187, 154, 247, 141, 35)),
            Self::Number => fg(ink(224, 175, 104, 179, 33)),
        };
        Spec { fg, bg, bold }
    }

    /// Full SGR sequence for this tone, or "" when it adds nothing.
    pub fn sgr(self, depth: ColorDepth) -> String {
        self.codes(depth, true)
    }

    /// SGR for this tone; without `reset` it layers on the current style.
    fn codes(self, depth: ColorDepth, reset: bool) -> String {
        let spec = self.spec();
        let mut codes = String::new();
        let mut code = |value: &str| {
            if !codes.is_empty() {
                codes.push(';');
            }
            codes.push_str(value);
        };
        if reset {
            code("0");
        }
        if spec.bold {
            code("1");
        }
        match depth {
            ColorDepth::None | ColorDepth::Ansi16 if self == Self::Cursor => code("7"),
            ColorDepth::None => {}
            ColorDepth::Ansi16 => {
                if let Some(fg) = spec.fg.filter(|fg| fg.ansi != 0) {
                    code(&fg.ansi.to_string());
                }
            }
            ColorDepth::Ansi256 => {
                if let Some(fg) = spec.fg {
                    code(&format!("38;5;{}", fg.xterm));
                }
                if let Some(bg) = spec.bg {
                    code(&format!("48;5;{}", bg.xterm));
                }
                if self == Self::Cursor {
                    code("38;5;234;48;5;189");
                }
            }
            ColorDepth::TrueColor => {
                if let Some(Ink { rgb: (r, g, b), .. }) = spec.fg {
                    code(&format!("38;2;{r};{g};{b}"));
                }
                if let Some(Ink { rgb: (r, g, b), .. }) = spec.bg {
                    code(&format!("48;2;{r};{g};{b}"));
                }
                if self == Self::Cursor {
                    code("38;2;26;27;38;48;2;192;202;245");
                }
            }
        }
        if codes.is_empty() || depth == ColorDepth::None && codes == "0" {
            return String::new();
        }
        let mut out = String::with_capacity(codes.len() + 3);
        let _ = write!(out, "\x1b[{codes}m");
        out
    }
}

/// Pad a row to `width` cells so background tones span the full line.
pub fn pad(mut row: Row, width: usize) -> Row {
    row.text = text::pad(&row.text, width);
    row
}

/// Prefix a plain (unstyled) margin; a background row keeps it uncolored.
pub fn indent(mut row: Row, margin: &str) -> Row {
    if margin.is_empty() {
        return row;
    }
    let shift = margin.len();
    row.text.insert_str(0, margin);
    for (range, _) in &mut row.spans {
        *range = range.start + shift..range.end + shift;
    }
    if row.tone != Tone::Text {
        row.spans.insert(0, (0..shift, Tone::Text));
    }
    row
}

#[cfg(test)]
#[path = "style_tests.rs"]
mod tests;
