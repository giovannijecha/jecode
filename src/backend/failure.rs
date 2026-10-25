use super::{Failure, FailureKind};

impl Failure {
    pub fn temporary(message: impl Into<String>) -> Self {
        Self {
            kind: FailureKind::Temporary,
            reason: None,
            message: message.into(),
            diagnostic: None,
            retry_after: None,
            charges: None,
        }
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self {
            kind: FailureKind::Terminal,
            reason: None,
            message,
            diagnostic: None,
            retry_after: None,
            charges: None,
        }
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}
