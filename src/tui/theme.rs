use std::fmt;

// Semantic roles; docs/tui/THEME.md records their use and contrast. Ordinary
// text and the general background inherit the terminal's defaults. Weight and
// slant are composed at the call site: `STRONG.bold()`, `MUTED.italic()`.

// Text, from strongest to faintest.
pub const BODY: Style = Style::INHERIT;
pub const STRONG: Style = Style::rgb(226, 232, 240); // #E2E8F0
pub const EMPHASIS: Style = Style::rgb(220, 228, 240); // #DCE4F0
pub const CODE_TEXT: Style = Style::rgb(201, 209, 217); // #C9D1D9
pub const MUTED: Style = Style::rgb(154, 164, 178); // #9AA4B2
pub const TOOL_DIM: Style = Style::rgb(135, 154, 170); // #879AAA

// Interaction and outcome.
pub const ACCENT: Style = Style::rgb(122, 162, 247); // #7AA2F7
pub const TOOL_ACCENT: Style = Style::rgb(153, 184, 212); // #99B8D4
pub const KEYWORD: Style = Style::rgb(187, 154, 247); // #BB9AF7
pub const GOOD: Style = Style::rgb(158, 206, 106); // #9ECE6A
pub const BAD: Style = Style::rgb(239, 139, 139); // #EF8B8B
pub const WARNING: Style = Style::rgb(224, 175, 104); // #E0AF68

// Catalog surfaces; code and tool selection keep their own fills.
pub const USER_SURFACE: Surface = Surface::rgb(40, 46, 57); // #282E39
pub const SELECTED_SURFACE: Surface = Surface::rgb(53, 61, 75); // #353D4B
pub const CODE_SURFACE: Surface = Surface::rgb(32, 37, 44); // #20252C
pub const DIFF_ADDED_SURFACE: Surface = Surface::rgb(30, 46, 35); // #1E2E23
pub const DIFF_REMOVED_SURFACE: Surface = Surface::rgb(49, 32, 38); // #312026

// The software caret carries its own fill, even inside a shaded panel.
pub const CURSOR: Style = Style::rgb(26, 27, 38).filled(192, 202, 245); // #1A1B26 / #C0CAF5
// Activity wave shades, from resting to brightest.
pub const WAVE_SHADES: [Style; 4] = [
    Style::rgb(78, 97, 113),
    Style::rgb(105, 133, 159),
    Style::rgb(137, 169, 196),
    Style::rgb(181, 213, 241),
];

// Component aliases, derived from the roles above.
pub const INLINE_CODE: Style = ACCENT;
pub const LINK: Style = ACCENT.underline();
pub const LINK_DESTINATION: Style = ACCENT;
pub const CODE_STRING: Style = GOOD;
pub const CODE_NUMBER: Style = WARNING;
pub const NOTICE_INFO: Style = MUTED.italic();
pub const NOTICE_WARNING: Style = WARNING.italic();
pub const NOTICE_ERROR: Style = BAD.italic();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rgb(u8, u8, u8);

// A foreground colour with SGR attributes, painted as one parameter list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    attributes: u8,
    foreground: Option<Rgb>,
    fill: Option<Rgb>,
}

impl Style {
    const INHERIT: Self = Self {
        attributes: 0,
        foreground: None,
        fill: None,
    };
    // SGR attribute codes in painting order.
    const ATTRIBUTES: [(u8, &'static str); 4] = [(1, "1"), (2, "3"), (4, "4"), (8, "9")];

    const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self {
            foreground: Some(Rgb(red, green, blue)),
            ..Self::INHERIT
        }
    }
    const fn filled(mut self, red: u8, green: u8, blue: u8) -> Self {
        self.fill = Some(Rgb(red, green, blue));
        self
    }
    const fn with(mut self, attribute: u8) -> Self {
        self.attributes |= attribute;
        self
    }
    pub const fn bold(self) -> Self {
        self.with(1)
    }
    pub const fn italic(self) -> Self {
        self.with(2)
    }
    pub const fn underline(self) -> Self {
        self.with(4)
    }
    pub const fn strike(self) -> Self {
        self.with(8)
    }
    // A style with its own fill ignores the surface beneath it.
    pub const fn has_fill(self) -> bool {
        self.fill.is_some()
    }
}

impl fmt::Display for Style {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut separator = "";
        for (attribute, code) in Self::ATTRIBUTES {
            if self.attributes & attribute != 0 {
                write!(formatter, "{separator}{code}")?;
                separator = ";";
            }
        }
        for (prefix, color) in [("38", self.foreground), ("48", self.fill)] {
            if let Some(Rgb(red, green, blue)) = color {
                write!(formatter, "{separator}{prefix};2;{red};{green};{blue}")?;
                separator = ";";
            }
        }
        if separator.is_empty() {
            formatter.write_str("0")?;
        }
        Ok(())
    }
}

// A background fill; `TERMINAL` restores the terminal's own background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface(Option<Rgb>);

impl Surface {
    pub const TERMINAL: Self = Self(None);

    const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self(Some(Rgb(red, green, blue)))
    }
}

impl fmt::Display for Surface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(Rgb(red, green, blue)) => write!(formatter, "48;2;{red};{green};{blue}"),
            None => formatter.write_str("49"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_paint_attributes_before_colors() {
        assert_eq!(BODY.to_string(), "0");
        assert_eq!(
            STRONG.bold().italic().strike().to_string(),
            "1;3;9;38;2;226;232;240"
        );
        assert_eq!(LINK.bold().to_string(), "1;4;38;2;122;162;247");
        assert_eq!(BODY.italic().strike().to_string(), "3;9");
        assert_eq!(CURSOR.to_string(), "38;2;26;27;38;48;2;192;202;245");
        assert_eq!(USER_SURFACE.to_string(), "48;2;40;46;57");
        assert_eq!(EMPHASIS.to_string(), "38;2;220;228;240");
        assert_eq!(TOOL_DIM.to_string(), "38;2;135;154;170");
        assert_eq!(SELECTED_SURFACE.to_string(), "48;2;53;61;75");
        assert_eq!(CODE_SURFACE.to_string(), "48;2;32;37;44");
        assert_eq!(Surface::TERMINAL.to_string(), "49");
    }
}
