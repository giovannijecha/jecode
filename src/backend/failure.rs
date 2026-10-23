use super::{Failure, FailureKind};

impl Failure {
    pub fn temporary(message: impl Into<String>) -> Self {
        Self {
            kind: FailureKind::Temporary,
            message: message.into(),
            retry_after: None,
            charges: None,
        }
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self {
            kind: FailureKind::Terminal,
            message,
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
