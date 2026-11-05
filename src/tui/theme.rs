#[cfg(any(unix, test))]
use std::ffi::OsString;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

// Semantic roles of the v2 design; docs/tui/THEME.md records their use and
// contrast. The renderer uses the terminal background on dark themes and a
// local dark fill when OSC 11 reports a light theme. Weight and slant are composed
// at the call site: `STRONG.bold()`, `MUTED.italic()`.

static PALETTE_ONLY: AtomicBool = AtomicBool::new(false);

/// Paint with the 256-colour palette unless the terminal declares 24-bit colour,
/// which others misread: Terminal.app before macOS 26 shows wrong colours. Windows
/// consoles at the supported floor all have it, so only Unix terminals ask.
#[cfg(unix)]
pub(in crate::tui) fn choose_color_depth(variable: impl Fn(&str) -> Option<OsString>) {
    PALETTE_ONLY.store(!declares_true_color(variable), Ordering::Relaxed);
}

#[cfg(any(unix, test))]
fn declares_true_color(variable: impl Fn(&str) -> Option<OsString>) -> bool {
    let value = |name: &str| {
        variable(name)
            .map(|value| value.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    };
    if matches!(value("COLORTERM").as_str(), "truecolor" | "24bit") {
        return true;
    }
    let term = value("TERM");
    if [
        "truecolor",
        "24bit",
        "direct",
        "kitty",
        "alacritty",
        "ghostty",
        "foot",
        "wezterm",
    ]
    .iter()
    .any(|name| term.contains(name))
    {
        return true;
    }
    if matches!(
        value("TERM_PROGRAM").as_str(),
        "iterm.app" | "wezterm" | "ghostty" | "vscode" | "hyper" | "tabby" | "warpterminal"
    ) {
        return true;
    }
    // These terminals have had 24-bit colour from before their identifying variable.
    [
        "WT_SESSION",
        "KONSOLE_VERSION",
        "KITTY_WINDOW_ID",
        "ALACRITTY_WINDOW_ID",
    ]
    .iter()
    .any(|name| variable(name).is_some())
        || value("VTE_VERSION")
            .parse::<u32>()
            .is_ok_and(|version| version >= 3600)
}

// Text, from strongest to faintest.
pub const BODY: Style = Style::rgb(212, 211, 219); // #D4D3DB
pub const STRONG: Style = Style::rgb(232, 230, 242); // #E8E6F2
pub const CODE_TEXT: Style = Style::rgb(201, 209, 217); // #C9D1D9
pub const MUTED: Style = Style::rgb(164, 161, 187); // #A4A1BB
pub const DIM: Style = Style::rgb(142, 138, 174); // #8E8AAE
// Marks only, never text: rules while working, borders and tracks (3:1).
pub const FAINT: Style = Style::rgb(107, 103, 150); // #6B6796

// Interaction and outcome.
pub const ACCENT: Style = Style::rgb(195, 177, 250); // #C3B1FA
pub const LIT: Style = Style::rgb(226, 217, 252); // #E2D9FC
pub const KEYWORD: Style = Style::rgb(224, 166, 242); // #E0A6F2
pub const GOOD: Style = Style::rgb(158, 206, 106); // #9ECE6A
pub const BAD: Style = Style::rgb(239, 139, 139); // #EF8B8B
pub const WARNING: Style = Style::rgb(224, 175, 104); // #E0AF68
pub const BADGE_TEXT: Style = Style::rgb(242, 245, 252); // #F2F5FC

// The activity shimmer: a five-cell band `S0 S1 S2 S1 S0` over `S_BASE`.
pub const S0: Style = Style::rgb(169, 155, 230); // #A99BE6
pub const S1: Style = Style::rgb(217, 203, 251); // #D9CBFB
pub const S2: Style = Style::rgb(251, 248, 255); // #FBF8FF
pub const S_BASE: Style = Style::rgb(110, 100, 160); // #6E64A0

// Surfaces, 1.2–1.6 contrast steps above a dark background. `BASE` is the
// terminal default token; the renderer may replace it locally on light themes.
pub const BASE: Surface = Surface(None);
pub const USER_SURFACE: Surface = Surface::rgb(41, 40, 63); // #29283F
pub const PILL_SURFACE: Surface = Surface::rgb(39, 38, 59); // #27263B
pub const SELECTED_SURFACE: Surface = Surface::rgb(56, 54, 86); // #383656
pub const CODE_SURFACE: Surface = Surface::rgb(32, 37, 44); // #20252C
pub const DIFF_ADDED_SURFACE: Surface = Surface::rgb(30, 46, 35); // #1E2E23
pub const DIFF_REMOVED_SURFACE: Surface = Surface::rgb(49, 32, 38); // #312026
pub const DIFF_ADDED_STRONG: Surface = Surface::rgb(44, 74, 51); // #2C4A33
pub const DIFF_REMOVED_STRONG: Surface = Surface::rgb(82, 48, 57); // #523039

// Tool badges: the family colour when finished, the state otherwise.
pub const BADGE_READ: Surface = Surface::rgb(76, 74, 178); // #4C4AB2
pub const BADGE_EDIT: Surface = Surface::rgb(122, 63, 158); // #7A3F9E
pub const BADGE_SHELL: Surface = Surface::rgb(61, 88, 163); // #3D58A3
pub const BADGE_COORD: Surface = Surface::rgb(45, 99, 99); // #2D6363
pub const BADGE_IDLE: Surface = Surface::rgb(52, 50, 79); // #34324F
pub const BADGE_BAD: Surface = Surface::rgb(122, 46, 58); // #7A2E3A
pub const BADGE_WARN: Surface = Surface::rgb(107, 82, 34); // #6B5222

// The software caret carries its own fill, even inside a shaded row.
pub const CURSOR: Style = Style::rgb(26, 27, 38).filled(192, 202, 245); // #1A1B26 / #C0CAF5

// Component aliases, derived from the roles above.
// Inline code is a chip on the code surface, apart from the link accent.
pub const INLINE_CODE: Style = CODE_TEXT;
pub const INLINE_CODE_SURFACE: Surface = CODE_SURFACE;
pub const LINK: Style = ACCENT.underline();
pub const LINK_DESTINATION: Style = MUTED;
pub const CODE_STRING: Style = GOOD;
pub const CODE_NUMBER: Style = WARNING;
pub const METER_FILL: Style = ACCENT;
pub const METER_TRACK: Style = FAINT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rgb(u8, u8, u8);

impl Rgb {
    /// The SGR parameters for this colour; `prefix` is 38 or 48.
    fn parameters(self, prefix: u8, palette_only: bool) -> String {
        let Self(red, green, blue) = self;
        if palette_only {
            format!("{prefix};5;{}", self.nearest_palette_index())
        } else {
            format!("{prefix};2;{red};{green};{blue}")
        }
    }

    /// The closest entry of the standard palette: its 6x6x6 cube or grey ramp. Dark
    /// tints that mean something would all turn grey, so they keep a chosen hue.
    fn nearest_palette_index(self) -> u8 {
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        const TINTS: [(Surface, u8); 7] = [
            (DIFF_ADDED_SURFACE, 22),
            (DIFF_ADDED_STRONG, 22),
            (DIFF_REMOVED_SURFACE, 52),
            (DIFF_REMOVED_STRONG, 52),
            (BADGE_SHELL, 25),
            (BADGE_COORD, 23),
            (BADGE_BAD, 88),
        ];
        if let Some((_, index)) = TINTS.iter().find(|(surface, _)| surface.0 == Some(self)) {
            return *index;
        }
        let Self(red, green, blue) = self;
        let level = |channel: u8| {
            (0..LEVELS.len())
                .min_by_key(|&index| LEVELS[index].abs_diff(channel))
                .unwrap_or(0)
        };
        let [r, g, b] = [red, green, blue].map(level);
        let cube = Self(LEVELS[r], LEVELS[g], LEVELS[b]);
        let average = (u16::from(red) + u16::from(green) + u16::from(blue)) / 3;
        let step = (average.saturating_sub(3) / 10).min(23) as u8;
        let grey_value = 8 + 10 * step;
        let distance = |Self(x, y, z): Self| {
            [(x, red), (y, green), (z, blue)]
                .into_iter()
                .map(|(from, to)| u32::from(from.abs_diff(to)).pow(2))
                .sum::<u32>()
        };
        if distance(Self(grey_value, grey_value, grey_value)) < distance(cube) {
            232 + step
        } else {
            16 + 36 * r as u8 + 6 * g as u8 + b as u8
        }
    }
}

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
        let palette_only = PALETTE_ONLY.load(Ordering::Relaxed);
        for (prefix, color) in [(38, self.foreground), (48, self.fill)] {
            if let Some(color) = color {
                let parameters = color.parameters(prefix, palette_only);
                write!(formatter, "{separator}{parameters}")?;
                separator = ";";
            }
        }
        if separator.is_empty() {
            formatter.write_str("0")?;
        }
        Ok(())
    }
}

// A background fill; `None` is the terminal's default background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface(Option<Rgb>);

impl Default for Surface {
    fn default() -> Self {
        BASE
    }
}

impl Surface {
    pub(in crate::tui) const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self(Some(Rgb(red, green, blue)))
    }
}

impl fmt::Display for Surface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(color) => {
                formatter.write_str(&color.parameters(48, PALETTE_ONLY.load(Ordering::Relaxed)))
            }
            None => formatter.write_str("49"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_paint_attributes_before_colors() {
        assert_eq!(BODY.to_string(), "38;2;212;211;219");
        assert_eq!(
            STRONG.bold().italic().strike().to_string(),
            "1;3;9;38;2;232;230;242"
        );
        assert_eq!(LINK.bold().to_string(), "1;4;38;2;195;177;250");
        assert_eq!(BODY.italic().strike().to_string(), "3;9;38;2;212;211;219");
        assert_eq!(CURSOR.to_string(), "38;2;26;27;38;48;2;192;202;245");
        assert_eq!(BASE.to_string(), "49");
        assert_eq!(USER_SURFACE.to_string(), "48;2;41;40;63");
        assert_eq!(SELECTED_SURFACE.to_string(), "48;2;56;54;86");
        assert_eq!(CODE_SURFACE.to_string(), "48;2;32;37;44");
        assert_eq!(FAINT.to_string(), "38;2;107;103;150");
    }

    fn luminance(Rgb(red, green, blue): Rgb) -> f64 {
        let linear = |channel: u8| {
            let channel = f64::from(channel) / 255.0;
            if channel <= 0.04045 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
    }

    fn contrast(foreground: Style, surface: Surface) -> f64 {
        let fore = luminance(foreground.foreground.unwrap());
        let back = luminance(surface.0.unwrap());
        (fore.max(back) + 0.05) / (fore.min(back) + 0.05)
    }

    #[test]
    fn text_roles_hold_their_contrast_on_dark_terminal_backgrounds() {
        // Campbell, the former painted base and Dark+; lighter dark themes
        // such as One Half Dark (#282C34) drop DIM and FAINT below the floors.
        let terminals = [(12, 12, 12), (23, 22, 31), (30, 30, 30)]
            .map(|(red, green, blue)| Surface::rgb(red, green, blue));
        for (role, floor) in [
            (STRONG, 13.0),
            (BODY, 11.0),
            (ACCENT, 8.5),
            (MUTED, 6.5),
            (DIM, 5.0),
            (GOOD, 4.5),
            (BAD, 4.5),
            (WARNING, 4.5),
        ] {
            for terminal in terminals {
                assert!(contrast(role, terminal) >= floor, "{role:?} {terminal:?}");
            }
        }
        // Marks meet the 3:1 floor for non-text contrast.
        for terminal in terminals {
            assert!(contrast(FAINT, terminal) >= 3.0, "{terminal:?}");
        }
        for surface in [
            BADGE_READ,
            BADGE_EDIT,
            BADGE_SHELL,
            BADGE_COORD,
            BADGE_BAD,
            BADGE_WARN,
        ] {
            assert!(contrast(BADGE_TEXT, surface) >= 4.5, "{surface:?}");
        }
        assert!(contrast(LIT, BADGE_IDLE) >= 4.5);
        assert!(contrast(STRONG, SELECTED_SURFACE) >= 4.5);
    }

    fn palette_color(index: u8) -> Rgb {
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        match index {
            16..=231 => {
                let index = usize::from(index - 16);
                Rgb(LEVELS[index / 36], LEVELS[index / 6 % 6], LEVELS[index % 6])
            }
            232..=255 => {
                let grey = 8 + 10 * (index - 232);
                Rgb(grey, grey, grey)
            }
            _ => panic!("the palette mapping must avoid the themeable first 16 colours"),
        }
    }

    #[test]
    fn colors_map_to_the_nearest_stable_palette_entry() {
        for (color, index) in [
            (Rgb(0, 0, 0), 16),
            (Rgb(255, 255, 255), 231),
            (Rgb(135, 95, 255), 99),
            (Rgb(8, 8, 8), 232),
            (Rgb(38, 38, 38), 235),
            (Rgb(238, 238, 238), 255),
        ] {
            assert_eq!(color.nearest_palette_index(), index, "{color:?}");
        }
        assert_eq!(Rgb(195, 177, 250).parameters(38, true), "38;5;147");
        assert_eq!(Rgb(195, 177, 250).parameters(48, false), "48;2;195;177;250");
    }

    #[test]
    fn the_palette_keeps_text_legible_on_every_surface() {
        let mapped = |Rgb(red, green, blue): Rgb| {
            palette_color(Rgb(red, green, blue).nearest_palette_index())
        };
        let contrast_after = |foreground: Style, surface: Surface| {
            let (fore, back) = (
                luminance(mapped(foreground.foreground.unwrap())),
                luminance(mapped(surface.0.unwrap())),
            );
            (fore.max(back) + 0.05) / (fore.min(back) + 0.05)
        };
        for (foreground, surface) in [
            (BODY, USER_SURFACE),
            (CODE_TEXT, CODE_SURFACE),
            (CODE_TEXT, DIFF_ADDED_SURFACE),
            (CODE_TEXT, DIFF_REMOVED_SURFACE),
            (MUTED, DIFF_ADDED_SURFACE),
            (MUTED, DIFF_REMOVED_SURFACE),
            (GOOD, DIFF_ADDED_STRONG),
            (BAD, DIFF_REMOVED_STRONG),
            (STRONG, SELECTED_SURFACE),
            (LIT, BADGE_IDLE),
            (BADGE_TEXT, BADGE_READ),
            (BADGE_TEXT, BADGE_EDIT),
            (BADGE_TEXT, BADGE_SHELL),
            (BADGE_TEXT, BADGE_COORD),
            (BADGE_TEXT, BADGE_BAD),
            (BADGE_TEXT, BADGE_WARN),
        ] {
            // The palette cannot hold the 24-bit floors exactly: 4 for text, and 3
            // for the gutter numbers, the only muted text on an added row.
            let gutter = foreground == MUTED && surface == DIFF_ADDED_SURFACE;
            let floor = if gutter { 3.0 } else { 4.0 };
            let ratio = contrast_after(foreground, surface);
            assert!(ratio >= floor, "{foreground:?} on {surface:?}: {ratio:.2}");
        }
        // Rows keep a visible surface instead of falling to the terminal's background.
        for surface in [
            USER_SURFACE,
            CODE_SURFACE,
            SELECTED_SURFACE,
            DIFF_ADDED_SURFACE,
            DIFF_REMOVED_SURFACE,
        ] {
            assert_ne!(mapped(surface.0.unwrap()), Rgb(0, 0, 0), "{surface:?}");
        }
        // Colour still separates added from removed, and failure from ordinary tools.
        assert_ne!(
            mapped(DIFF_ADDED_SURFACE.0.unwrap()),
            mapped(DIFF_REMOVED_SURFACE.0.unwrap())
        );
        assert_ne!(mapped(BADGE_BAD.0.unwrap()), mapped(BADGE_SHELL.0.unwrap()));
    }

    #[test]
    fn only_a_declared_terminal_receives_true_color() {
        let declares = |pairs: &[(&str, &str)]| {
            declares_true_color(|name| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| OsString::from(value))
            })
        };
        for pairs in [
            &[("COLORTERM", "truecolor")][..],
            &[("COLORTERM", "24bit"), ("TERM", "xterm")],
            &[("TERM", "xterm-kitty")],
            &[("TERM", "xterm-direct")],
            &[("TERM_PROGRAM", "iTerm.app")],
            &[("TERM_PROGRAM", "vscode")],
            &[("WT_SESSION", "id")],
            &[("VTE_VERSION", "7600")],
        ] {
            assert!(declares(pairs), "{pairs:?}");
        }
        for pairs in [
            &[][..],
            &[("TERM", "xterm-256color")],
            &[
                ("TERM", "xterm-256color"),
                ("TERM_PROGRAM", "Apple_Terminal"),
            ],
            &[("TERM", "screen-256color")],
            &[("TERM", "linux")],
            &[("COLORTERM", "yes")],
            &[("VTE_VERSION", "3400")],
        ] {
            assert!(!declares(pairs), "{pairs:?}");
        }
    }

    #[test]
    fn tinted_diff_rows_keep_text_and_changed_fragments_legible() {
        for (foreground, surface) in [
            (CODE_TEXT, DIFF_ADDED_SURFACE),
            (CODE_TEXT, DIFF_REMOVED_SURFACE),
            (MUTED, DIFF_ADDED_SURFACE),
            (MUTED, DIFF_REMOVED_SURFACE),
            (GOOD, DIFF_ADDED_STRONG),
            (BAD, DIFF_REMOVED_STRONG),
        ] {
            assert!(contrast(foreground, surface) >= 4.5);
        }
        assert_eq!(DIFF_ADDED_STRONG.to_string(), "48;2;44;74;51");
        assert_eq!(DIFF_REMOVED_STRONG.to_string(), "48;2;82;48;57");
    }
}
