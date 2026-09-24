use crate::providers::openai_account::{Usage, client};
use std::fmt;

/// One applied selection. Fixed storage keeps validated identifiers cheap to copy
/// without retaining a catalog (or its account metadata) in a saved session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    id: [u8; 128],
    id_len: u8,
    effort: [u8; 32],
    effort_len: u8,
}
impl Model {
    #[allow(non_upper_case_globals)]
    pub const Luna: Self = Self::historical(b"gpt-5.6-luna");
    #[allow(non_upper_case_globals)]
    pub const Terra: Self = Self::historical(b"gpt-5.6-terra");

    const fn historical(id: &[u8]) -> Self {
        let mut value = Self {
            id: [0; 128],
            id_len: id.len() as u8,
            effort: [0; 32],
            effort_len: 6,
        };
        let mut index = 0;
        while index < id.len() {
            value.id[index] = id[index];
            index += 1;
        }
        value.effort[0] = b'm';
        value.effort[1] = b'e';
        value.effort[2] = b'd';
        value.effort[3] = b'i';
        value.effort[4] = b'u';
        value.effort[5] = b'm';
        value
    }

    /// `None` means omit the effort field and let the account provider decide.
    pub fn new(id: &str, effort: Option<&str>) -> Option<Self> {
        if !valid_identifier(id, 128) || effort.is_some_and(|value| !valid_identifier(value, 32)) {
            return None;
        }
        let mut value = Self {
            id: [0; 128],
            id_len: id.len() as u8,
            effort: [0; 32],
            effort_len: 0,
        };
        value.id[..id.len()].copy_from_slice(id.as_bytes());
        if let Some(effort) = effort {
            value.effort[..effort.len()].copy_from_slice(effort.as_bytes());
            value.effort_len = effort.len() as u8;
        }
        Some(value)
    }

    pub fn id(&self) -> &str {
        std::str::from_utf8(&self.id[..usize::from(self.id_len)]).expect("validated ASCII")
    }

    pub fn effort(&self) -> Option<&str> {
        (self.effort_len != 0).then(|| {
            std::str::from_utf8(&self.effort[..usize::from(self.effort_len)])
                .expect("validated ASCII")
        })
    }

    pub fn with_effort(self, effort: Option<&str>) -> Option<Self> {
        Self::new(self.id(), effort)
    }
}

fn valid_identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Account(client::Error),
    Cancelled,
    HistoryLimit,
    CompactionOutput,
    CompactionIneffective,
    OutputLimit,
    UnexpectedTools,
    Worker,
    Storage,
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Account(error) => error.fmt(f),
            Self::Cancelled => f.write_str("Interrupted / partial output retained"),
            Self::HistoryLimit => f.write_str("Context cannot be reduced at a completed boundary within the request budget / use /compact explicitly after reducing the input"),
            Self::CompactionOutput => f.write_str("Compaction returned an incomplete, empty or invalid summary / prior context retained"),
            Self::CompactionIneffective => f.write_str("Compaction did not reduce the projected request / prior context retained"),
            Self::OutputLimit => {
                f.write_str("Response display limit reached / partial output retained")
            }
            Self::UnexpectedTools => f.write_str("Unexpected tool request / no tool was executed"),
            Self::Worker => f.write_str("Account worker stopped unexpectedly"),
            Self::Storage => {
                f.write_str("Session could not be saved / stopped before further work")
            }
        }
    }
}
impl Failure {
    pub(crate) fn needs_login(self) -> bool {
        matches!(
            self,
            Self::Account(
                client::Error::Expired
                    | client::Error::AccountChanged
                    | client::Error::Login(crate::providers::openai_account::auth::Error::Denied)
            )
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    Complete,
    Incomplete,
    Refused,
    Failed(Failure),
}
impl End {
    pub(crate) fn needs_login(self) -> bool {
        matches!(self, Self::Failed(failure) if failure.needs_login())
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Metrics {
    pub requests: u32,
    pub connection_attempts: u32,
    pub submissions: u32,
    pub tool_calls: u32,
    pub elapsed_ms: u64,
    pub approval_wait_ms: u64,
    pub first_text_ms: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}
impl Metrics {
    pub(super) fn observe(&mut self, attempt: &client::Attempt) {
        self.connection_attempts = self.connection_attempts.saturating_add(1);
        if attempt.delivery != client::Delivery::NotSubmitted {
            self.submissions = self.submissions.saturating_add(1);
        }
    }
    pub(super) fn usage(&mut self, usage: &Usage) {
        let add = |old: Option<u64>, new: Option<u64>| {
            if self.requests == 1 {
                new
            } else {
                old.zip(new).and_then(|(a, b)| a.checked_add(b))
            }
        };
        self.input_tokens = add(self.input_tokens, usage.input);
        self.output_tokens = add(self.output_tokens, usage.output);
        self.cached_tokens = add(self.cached_tokens, usage.cached);
        self.reasoning_tokens = add(self.reasoning_tokens, usage.reasoning);
    }
}

pub enum Event {
    Guidance {
        text: String,
        new_turn: bool,
    },
    GuidanceReturned(String),
    ContextReport(String),
    Restored {
        id: String,
        items: Vec<TranscriptItem>,
        turns: usize,
    },
    LoginCode(String),
    Ready,
    CatalogLoaded(crate::providers::openai_account::catalog::Catalog),
    CatalogFailed(CatalogFailure),
    LoggedOut,
    LogoutFailed(Failure, bool),
    ModelChanged(Model),
    Thinking,
    RequestStarted,
    Retrying,
    ToolStarted {
        name: &'static str,
        path: String,
    },
    ToolFinished {
        summary: String,
        failed: bool,
        limited: bool,
    },
    EditProposed {
        id: u64,
        preview: crate::workspace::Preview,
    },
    EditFinished {
        id: u64,
        summary: String,
        applied: bool,
        failed: bool,
    },
    CommandProposed {
        id: u64,
        preview: crate::command::Preview,
    },
    CommandStarted {
        id: u64,
    },
    CommandOutput {
        id: u64,
        channel: crate::command::Channel,
        text: String,
    },
    CommandFinished {
        id: u64,
        summary: String,
        success: bool,
        failed: bool,
    },
    Text(String),
    TextReconciled(String),
    Finished(End, Metrics),
    LoginFailed(Failure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogFailure {
    Unavailable,
    Invalid,
    Empty,
    Cancelled,
}
pub struct TranscriptItem {
    pub role: &'static str,
    pub text: String,
}
