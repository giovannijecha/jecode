//! How long a provider keeps the cached prompt between requests.
//!
//! The preference reaches adapters as a request-only field on the system
//! message, so every agent, collaborator and backend reads the current value
//! at request time without a separate channel.

use crate::json::Value;

/// The request-only field naming the conversation's retention.
pub const MARKER: &str = "jecode_cache_retention";

/// Saved as `context.prompt_cache`. Adapters with explicit cache lifetimes
/// (Claude models) apply it; others keep their provider's automatic caching.
/// Five minutes is the default: one-hour writes cost 2× against 1.25×, and a new
/// turn's digest leaves little to rewrite after a break.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CacheRetention {
    /// The provider default: five minutes, renewed by every request.
    #[default]
    Short,
    /// One hour: the cache survives breaks; writes cost more.
    Long,
}

impl CacheRetention {
    pub const ALL: [Self; 2] = [Self::Short, Self::Long];

    pub fn id(self) -> &'static str {
        match self {
            Self::Short => "5m",
            Self::Long => "1h",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Short => "5 minutes",
            Self::Long => "1 hour",
        }
    }

    pub fn fact(self) -> &'static str {
        match self {
            Self::Short => "Cheapest without breaks",
            Self::Long => "Survives breaks · cache writes cost more",
        }
    }

    pub fn parse(id: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|retention| retention.id() == id)
            .ok_or_else(|| "Prompt cache must be \"5m\" or \"1h\"".to_string())
    }

    /// What a cache write costs against uncached input, for adapters that charge
    /// by lifetime. Decides how soon rewriting the window pays for itself.
    pub fn write_premium(self) -> f64 {
        match self {
            Self::Short => 1.25,
            Self::Long => 2.0,
        }
    }

    /// The retention a request asks for, read from its system messages.
    /// Requests without the marker, such as standalone maintenance, keep the
    /// provider default of five minutes.
    pub fn requested(messages: &[Value]) -> Self {
        let long = messages.iter().any(|message| {
            matches!(
                message.get("role").and_then(Value::as_str),
                Some("system" | "developer")
            ) && message.get(MARKER).and_then(Value::as_str) == Some(Self::Long.id())
        });
        if long { Self::Long } else { Self::Short }
    }
}
