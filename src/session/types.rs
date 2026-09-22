use crate::providers::openai_account::{Usage, client};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Luna,
    Terra,
}
impl Model {
    pub fn id(self) -> &'static str {
        match self {
            Self::Luna => "gpt-5.6-luna",
            Self::Terra => "gpt-5.6-terra",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Account(client::Error),
    Cancelled,
    HistoryLimit,
    OutputLimit,
    UnexpectedTools,
    StepLimit,
    Worker,
    Storage,
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Account(error) => error.fmt(f),
            Self::Cancelled => f.write_str("Interrupted / partial output retained"),
            Self::HistoryLimit => f.write_str(
                "Context limit reached / use /compact explicitly or start a new session",
            ),
            Self::OutputLimit => {
                f.write_str("Response display limit reached / partial output retained")
            }
            Self::UnexpectedTools => f.write_str("Unexpected tool request / no tool was executed"),
            Self::StepLimit => f.write_str("Turn limit reached / completed results retained"),
            Self::Worker => f.write_str("Account worker stopped unexpectedly"),
            Self::Storage => {
                f.write_str("Session could not be saved / stopped before further work")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    Complete,
    Incomplete,
    Refused,
    Failed(Failure),
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Metrics {
    pub requests: u32,
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
    Thinking,
    RequestStarted,
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
    Finished(End, Metrics),
    LoginFailed(Failure),
}
pub struct TranscriptItem {
    pub role: &'static str,
    pub text: String,
}
