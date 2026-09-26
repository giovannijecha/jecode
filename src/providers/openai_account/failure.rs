//! Fixed, safe classifications for provider-reported stream failures.
use crate::json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureEvent {
    ResponseFailed,
    Error,
}
impl FailureEvent {
    pub fn name(self) -> &'static str {
        match self {
            Self::ResponseFailed => "response.failed",
            Self::Error => "error",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "response.failed" => Self::ResponseFailed,
            "error" => Self::Error,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureCode {
    Unknown,
    Malformed,
    ServerError,
    RateLimitExceeded,
    SlowDown,
    ServerIsOverloaded,
    ContextLengthExceeded,
    InsufficientQuota,
    CreditBalanceExhausted,
    OrganizationSpendLimitExceeded,
    ProjectSpendLimitExceeded,
    OrganizationUsageLimitExceeded,
    UsageNotIncluded,
    InvalidPrompt,
}
impl FailureCode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Malformed => "malformed",
            Self::ServerError => "server_error",
            Self::RateLimitExceeded => "rate_limit_exceeded",
            Self::SlowDown => "slow_down",
            Self::ServerIsOverloaded => "server_is_overloaded",
            Self::ContextLengthExceeded => "context_length_exceeded",
            Self::InsufficientQuota => "insufficient_quota",
            Self::CreditBalanceExhausted => "credit_balance_exhausted",
            Self::OrganizationSpendLimitExceeded => "organization_spend_limit_exceeded",
            Self::ProjectSpendLimitExceeded => "project_spend_limit_exceeded",
            Self::OrganizationUsageLimitExceeded => "organization_usage_limit_exceeded",
            Self::UsageNotIncluded => "usage_not_included",
            Self::InvalidPrompt => "invalid_prompt",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "unknown" => Self::Unknown,
            "malformed" => Self::Malformed,
            "server_error" => Self::ServerError,
            "rate_limit_exceeded" => Self::RateLimitExceeded,
            "slow_down" => Self::SlowDown,
            "server_is_overloaded" => Self::ServerIsOverloaded,
            "context_length_exceeded" => Self::ContextLengthExceeded,
            "insufficient_quota" => Self::InsufficientQuota,
            "credit_balance_exhausted" => Self::CreditBalanceExhausted,
            "organization_spend_limit_exceeded" => Self::OrganizationSpendLimitExceeded,
            "project_spend_limit_exceeded" => Self::ProjectSpendLimitExceeded,
            "organization_usage_limit_exceeded" => Self::OrganizationUsageLimitExceeded,
            "usage_not_included" => Self::UsageNotIncluded,
            "invalid_prompt" => Self::InvalidPrompt,
            _ => return None,
        })
    }
    fn from_wire(value: Option<&Value>) -> Self {
        match value {
            None | Some(Value::Null) => Self::Unknown,
            Some(Value::String(code))
                if code.len() <= 64 && !code.chars().any(char::is_control) =>
            {
                Self::parse(code)
                    .filter(|code| *code != Self::Malformed)
                    .unwrap_or(Self::Unknown)
            }
            _ => Self::Malformed,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderFailure {
    pub event: FailureEvent,
    pub code: FailureCode,
}
impl ProviderFailure {
    pub(super) fn from_event(event: FailureEvent, data: &Value) -> Self {
        // Responses streaming specifies response.failed at response.error.code
        // and error events at top-level code. Never use message or a fallback
        // from another shape. Unknown provider strings are discarded here.
        let code = match event {
            FailureEvent::Error => FailureCode::from_wire(data.get("code")),
            FailureEvent::ResponseFailed => match data.get("response") {
                None | Some(Value::Null) => FailureCode::Unknown,
                Some(Value::Object(response)) => match response.get("error") {
                    None | Some(Value::Null) => FailureCode::Unknown,
                    Some(Value::Object(error)) => FailureCode::from_wire(error.get("code")),
                    _ => FailureCode::Malformed,
                },
                _ => FailureCode::Malformed,
            },
        };
        Self { event, code }
    }
}
