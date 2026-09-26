//! Terminal capabilities, detected once from the environment. Detection is a
//! pure function of an env lookup so every rule is testable.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorDepth {
    /// `NO_COLOR` or a dumb terminal: attributes only (bold, reverse).
    None,
    Ansi16,
    Ansi256,
    TrueColor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Caps {
    pub color: ColorDepth,
    /// Plain ASCII glyphs instead of box drawing, braille and symbols.
    pub ascii: bool,
    pub reduced_motion: bool,
}

impl Caps {
    /// `windows` selects the default when nothing is advertised: the modern
    /// Windows console renders truecolor, an unknown Unix TERM gets 16 colors.
    pub fn detect(env: impl Fn(&str) -> Option<String>, windows: bool) -> Self {
        let set = |key: &str| env(key).is_some_and(|value| !value.is_empty());
        let flag = |key: &str| env(key).is_some_and(|value| !value.is_empty() && value != "0");
        let term = env("TERM").unwrap_or_default();
        let color = if set("NO_COLOR") || term == "dumb" {
            ColorDepth::None
        } else if let Some(depth) = env("JECODE_COLOR").and_then(|value| parse_depth(&value)) {
            depth
        } else if env("COLORTERM")
            .is_some_and(|value| matches!(value.as_str(), "truecolor" | "24bit"))
            || set("WT_SESSION")
            || term.contains("direct")
        {
            ColorDepth::TrueColor
        } else if term.contains("256color") {
            ColorDepth::Ansi256
        } else if windows && term.is_empty() {
            ColorDepth::TrueColor
        } else {
            ColorDepth::Ansi16
        };
        let ascii = flag("JECODE_ASCII") || matches!(term.as_str(), "linux" | "vt100" | "vt220");
        Self {
            color,
            ascii,
            reduced_motion: flag("JECODE_REDUCED_MOTION"),
        }
    }
}

/// Accepts the `JECODE_COLOR` / `--color` spellings.
pub fn parse_depth(value: &str) -> Option<ColorDepth> {
    match value {
        "truecolor" | "24bit" => Some(ColorDepth::TrueColor),
        "256" => Some(ColorDepth::Ansi256),
        "16" => Some(ColorDepth::Ansi16),
        "none" => Some(ColorDepth::None),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(pairs: &[(&str, &str)], windows: bool) -> Caps {
        Caps::detect(
            |key| {
                pairs
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| value.to_string())
            },
            windows,
        )
    }

    #[test]
    fn no_color_wins_over_everything() {
        let caps = detect(
            &[
                ("NO_COLOR", "1"),
                ("JECODE_COLOR", "truecolor"),
                ("COLORTERM", "truecolor"),
            ],
            false,
        );
        assert_eq!(caps.color, ColorDepth::None);
    }

    #[test]
    fn empty_no_color_is_ignored() {
        assert_eq!(
            detect(&[("NO_COLOR", ""), ("COLORTERM", "24bit")], false).color,
            ColorDepth::TrueColor
        );
    }

    #[test]
    fn depth_ladder() {
        assert_eq!(
            detect(&[("JECODE_COLOR", "16"), ("COLORTERM", "truecolor")], false).color,
            ColorDepth::Ansi16
        );
        assert_eq!(
            detect(&[("WT_SESSION", "x")], true).color,
            ColorDepth::TrueColor
        );
        assert_eq!(
            detect(&[("TERM", "xterm-256color")], false).color,
            ColorDepth::Ansi256
        );
        assert_eq!(detect(&[("TERM", "dumb")], true).color, ColorDepth::None);
        assert_eq!(detect(&[], true).color, ColorDepth::TrueColor);
        assert_eq!(
            detect(&[("TERM", "xterm")], false).color,
            ColorDepth::Ansi16
        );
    }

    #[test]
    fn ascii_and_motion_flags() {
        let caps = detect(
            &[("JECODE_ASCII", "1"), ("JECODE_REDUCED_MOTION", "0")],
            false,
        );
        assert!(caps.ascii && !caps.reduced_motion);
        assert!(detect(&[("TERM", "linux")], false).ascii);
        assert!(detect(&[("JECODE_REDUCED_MOTION", "yes")], false).reduced_motion);
    }
}
