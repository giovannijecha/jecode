//! Which inks the interface draws with. It never paints a background, so a choice that
//! does not match the terminal reads badly rather than drawing a rectangle.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    /// Follows the terminal's reported background, dark when it does not answer.
    #[default]
    Auto,
    Dark,
    Light,
}

impl Theme {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Dark, Self::Light];

    pub fn id(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }

    pub fn fact(self) -> &'static str {
        match self {
            Self::Auto => "Follow the terminal's background",
            Self::Dark => "Dark inks, whatever the terminal says",
            Self::Light => "Light inks, whatever the terminal says",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|theme| theme.id() == value)
            .ok_or_else(|| "Theme must be \"auto\", \"dark\" or \"light\"".into())
    }
}
