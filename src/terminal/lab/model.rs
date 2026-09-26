//! Typed transcript model. Views match on the variant, never on speaker
//! strings or line prefixes, so presentation cannot drift from meaning.

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Block {
    /// A submitted prompt, shown verbatim.
    User(String),
    /// Assistant prose in the Markdown subset of `markdown.rs`.
    Assistant(String),
    /// Consecutive tool steps, drawn as one tree.
    Tools(Vec<Tool>),
    /// A slash command and its receipt.
    Command(Receipt),
    /// Local status or failure, never attributed to the assistant.
    Local { text: String, failed: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Running,
    Done,
    /// Finished, but not as asked (e.g. nothing matched, value kept).
    Warned,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Receipt {
    /// The command line as submitted, e.g. `/model son`.
    pub input: String,
    pub status: Status,
    /// One-line outcome, e.g. `Model set to sonnet-5`.
    pub result: String,
    /// Muted context after the result, e.g. `was opus-5.5`; may be empty.
    pub note: String,
    /// Aligned key/value rows under the result (`/help`, `/status`).
    pub facts: Vec<(String, String)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tool {
    /// What the step does: `Read`, `Run`, `Edit`, `Search`.
    pub verb: String,
    /// What it acts on: a path, a command line, a pattern.
    pub subject: String,
    /// Short result fact, e.g. `124 lines` or `exit 0`; may be empty.
    pub summary: String,
    pub status: Status,
    pub elapsed_ms: u64,
    pub detail: Detail,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Detail {
    /// Captured output; the view keeps its tail.
    Output(String),
    /// Unified-diff lines (`+`, `-`, ` `, `@@`); the view keeps its head.
    Diff(String),
}

/// A standing message above the composer (provider down, rate limited),
/// until the condition clears. `status` picks the mark: Warned or Failed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notice {
    pub status: Status,
    /// `headline · detail`; the detail after ` · ` is muted.
    pub text: String,
}
