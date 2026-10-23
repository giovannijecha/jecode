#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Effort {
    #[default]
    Default,
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub const LEVELS: [Self; 7] = [
        Self::Max,
        Self::Xhigh,
        Self::High,
        Self::Medium,
        Self::Low,
        Self::Minimal,
        Self::None,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        std::iter::once(Self::Default)
            .chain(Self::LEVELS)
            .find(|effort| effort.name().eq_ignore_ascii_case(value.trim()))
            .ok_or_else(|| {
                "Unknown effort. Use default, none, minimal, low, medium, high, xhigh or max."
                    .into()
            })
    }

    /// Whether requests ask for reasoning explicitly, which endpoints must accept.
    pub fn reasons(self) -> bool {
        !matches!(self, Self::Default | Self::None)
    }
}
