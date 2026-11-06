#[cfg(any(unix, test))]
use std::ffi::OsString;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

// Semantic roles of the design; docs/tui/THEME.md records their use and contrast.
// Every role carries its ink for dark and for light terminals, and painting picks
// one, so the call sites never change with the theme. Jecode never paints the
// terminal's background: cells without a surface keep SGR 49 in both. Weight and slant
// are composed at the call site: `STRONG.bold()`, `MUTED.italic()`.

static PALETTE_ONLY: AtomicBool = AtomicBool::new(false);

#[cfg(not(test))]
static LIGHT: AtomicBool = AtomicBool::new(false);

// Tests paint in parallel, so each keeps its own choice.
#[cfg(test)]
thread_local! {
    static LIGHT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(not(test))]
fn light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

#[cfg(test)]
fn light() -> bool {
    LIGHT.with(std::cell::Cell::get)
}

/// Selects the inks for a light or dark terminal; true when that changed them, so
/// the screen has to be painted again.
pub(in crate::tui) fn set_light(value: bool) -> bool {
    let previous = light();
    #[cfg(not(test))]
    LIGHT.store(value, Ordering::Relaxed);
    #[cfg(test)]
    LIGHT.with(|cell| cell.set(value));
    previous != value
}

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
pub const BODY: Style = Style::ink(0xD1D2D6, 0x2C2E31);
pub const STRONG: Style = Style::ink(0xEFF0F3, 0x191B1D);
pub const CODE_TEXT: Style = Style::ink(0xD1D2D6, 0x2C2E31);
pub const MUTED: Style = Style::ink(0xA4A6AA, 0x515256);
pub const DIM: Style = Style::ink(0x8C8E92, 0x616367);
// Marks only, never text: table borders, scrollbar and gauge tracks (3:1).
pub const FAINT: Style = Style::ink(0x6D6F72, 0x84868A);
// Decorative rules, which carry no meaning and so have no contrast floor.
pub const HAIRLINE: Style = Style::ink(0x3B3D40, 0xCFD1D5);

// Interaction and outcome. Red, amber and green only ever report a state.
pub const ACCENT: Style = Style::ink(0xC7AFFF, 0x552B9A);
pub const LIT: Style = Style::ink(0xDACEFF, 0x48198A);
pub const KEYWORD: Style = Style::ink(0xC8B7FF, 0x553494);
pub const GOOD: Style = Style::ink(0x7CD992, 0x066F32);
pub const BAD: Style = Style::ink(0xF48684, 0xB32228);
pub const WARNING: Style = Style::ink(0xEDC06B, 0x875500);
pub const BADGE_TEXT: Style = Style::ink(0xEFF0F3, 0xEFF0F3);

// The activity shimmer: a five-cell band `S0 S1 S2 S1 S0` over `S_BASE`.
pub const S0: Style = Style::ink(0xC7AFFF, 0x7250BA);
pub const S1: Style = Style::ink(0xDACEFF, 0x522797);
pub const S2: Style = Style::ink(0xFFFFFF, 0x27124B);
pub const S_BASE: Style = Style::ink(0x75689C, 0x9C8DCA);

// Surfaces mean something: a request, a selection, code. `BASE` is the terminal's own
// background, which Jecode never replaces.
pub const BASE: Surface = Surface(None);
pub const USER_SURFACE: Surface = Surface::ink(0x292A2F, 0xE6E8EC);
pub const SELECTED_SURFACE: Surface = Surface::ink(0x3A3B42, 0xD8DBE0);
pub const CODE_SURFACE: Surface = Surface::ink(0x212226, 0xEEF0F4);
pub const DIFF_ADDED_SURFACE: Surface = Surface::ink(0x1A2A1F, 0xDCF2DF);
pub const DIFF_REMOVED_SURFACE: Surface = Surface::ink(0x312026, 0xFFE4E4);
pub const DIFF_ADDED_STRONG: Surface = Surface::ink(0x2C4A33, 0xC8ECCE);
pub const DIFF_REMOVED_STRONG: Surface = Surface::ink(0x523039, 0xFFD5D5);

// Tool badges: the family colour when finished, the state otherwise. Family surfaces
// and their labels stay the same on either terminal; the quiet one follows the theme.
pub const BADGE_READ: Surface = Surface::ink(0x4852A0, 0x4852A0);
pub const BADGE_EDIT: Surface = Surface::ink(0x7F3C93, 0x7F3C93);
pub const BADGE_SHELL: Surface = Surface::ink(0x036387, 0x036387);
pub const BADGE_COORD: Surface = Surface::ink(0x226558, 0x226558);
pub const BADGE_IDLE: Surface = Surface::ink(0x3A3B42, 0xD8DBE0);
pub const BADGE_BAD: Surface = Surface::ink(0x7A2E3A, 0x7A2E3A);
pub const BADGE_WARN: Surface = Surface::ink(0x6B5222, 0x6B5222);

// The software caret carries its own fill, even inside a shaded row.
pub const CURSOR: Style = Style::ink(0x0C0C0C, 0xFFFFFF).filled(0xEFF0F3, 0x191B1D);

// Component aliases, derived from the roles above.
// Inline code is a chip on the code surface, apart from the link accent.
pub const INLINE_CODE: Style = CODE_TEXT;
pub const INLINE_CODE_SURFACE: Surface = CODE_SURFACE;
pub const LINK: Style = ACCENT.underline();
pub const LINK_DESTINATION: Style = MUTED;
// Code has its own inks, so green and amber only report states.
pub const CODE_STRING: Style = Style::ink(0x99D3DA, 0x016770);
pub const CODE_NUMBER: Style = Style::ink(0xE6B2CF, 0x943171);
pub const METER_FILL: Style = ACCENT;
pub const METER_TRACK: Style = FAINT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rgb(u8, u8, u8);

// One role's colour on each kind of terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ink {
    dark: Rgb,
    light: Rgb,
}

impl Ink {
    const fn new(dark: u32, light: u32) -> Self {
        Self {
            dark: Rgb::hex(dark),
            light: Rgb::hex(light),
        }
    }

    fn current(self) -> Rgb {
        if light() { self.light } else { self.dark }
    }
}

impl Rgb {
    const fn hex(value: u32) -> Self {
        Self((value >> 16) as u8, (value >> 8) as u8, value as u8)
    }

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
        if let Some((_, index)) = TINTS
            .iter()
            .find(|(surface, _)| surface.0.is_some_and(|ink| ink.dark == self))
        {
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
    foreground: Option<Ink>,
    fill: Option<Ink>,
}

impl Style {
    const INHERIT: Self = Self {
        attributes: 0,
        foreground: None,
        fill: None,
    };
    // SGR attribute codes in painting order.
    const ATTRIBUTES: [(u8, &'static str); 4] = [(1, "1"), (2, "3"), (4, "4"), (8, "9")];

    const fn ink(dark: u32, light: u32) -> Self {
        Self {
            foreground: Some(Ink::new(dark, light)),
            ..Self::INHERIT
        }
    }
    const fn filled(mut self, dark: u32, light: u32) -> Self {
        self.fill = Some(Ink::new(dark, light));
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
                let parameters = color.current().parameters(prefix, palette_only);
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
pub struct Surface(Option<Ink>);

impl Default for Surface {
    fn default() -> Self {
        BASE
    }
}

impl Surface {
    const fn ink(dark: u32, light: u32) -> Self {
        Self(Some(Ink::new(dark, light)))
    }
}

impl fmt::Display for Surface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(color) => formatter.write_str(
                &color
                    .current()
                    .parameters(48, PALETTE_ONLY.load(Ordering::Relaxed)),
            ),
            None => formatter.write_str("49"),
        }
    }
}

#[cfg(test)]
mod tests;
