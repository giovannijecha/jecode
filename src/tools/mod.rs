//! Bounded workspace tools. The controller alone orders and executes effects.
mod args;
mod read;
mod schema;
mod search;

use crate::{
    json::{self, Value},
    workspace::{Budget, Workspace},
};
pub use args::Prepared;
pub use schema::definitions;
pub(crate) use schema::definitions_for;

pub const MAX_OUTPUT: usize = 32 * 1024;
pub struct Output {
    pub text: String,
    pub summary: String,
    pub failed: bool,
    /// Successful output with truncation or omitted entries.
    pub limited: bool,
    pub stop_after: bool,
}
impl Output {
    pub fn error(message: &str) -> Self {
        Self {
            text: json::encode(
                &json::object([("ok", Value::Bool(false)), ("error", string(message))]),
                MAX_OUTPUT,
            )
            .expect("bounded owned tool error"),
            summary: message.into(),
            failed: true,
            limited: false,
            stop_after: false,
        }
    }
    pub(crate) fn success(value: Value, summary: String, limited: bool) -> Self {
        match json::encode(&value, MAX_OUTPUT) {
            Ok(text) => Self {
                text,
                summary,
                failed: false,
                limited,
                stop_after: false,
            },
            Err(_) => Self::error("tool result exceeded its output limit; narrow the request"),
        }
    }
    pub(crate) fn not_executed(message: &str) -> Self {
        let mut output = Self::success(
            json::object([
                ("ok", Value::Bool(false)),
                ("status", string("not_executed")),
                ("executed", Value::Bool(false)),
                ("error", string(message)),
            ]),
            message.into(),
            false,
        );
        output.failed = true;
        output
    }
    pub(crate) fn failed_effect_with_recovery(message: &str, recovery: Option<&str>) -> Self {
        let mut output = Self::success(
            json::object([
                ("ok", Value::Bool(false)),
                ("status", string("failed")),
                ("error", string(message)),
                ("recovery", recovery.map_or(Value::Null, string)),
            ]),
            message.into(),
            false,
        );
        output.failed = true;
        output
    }
}
impl Prepared {
    pub fn execute(&self, workspace: &Workspace, budget: &Budget<'_>) -> Output {
        let result = match self {
            Self::Command { .. } => {
                return Output::error("commands must execute through the session controller");
            }
            Self::Create { .. } | Self::Edit { .. } => {
                return Output::error("file changes must execute through the session controller");
            }
            Self::List { path, limit } => read::list(workspace, path, *limit, budget),
            Self::Read { path, start, lines } => {
                read::read(workspace, path, *start, *lines, budget)
            }
            Self::Search { path, query, limit } => {
                search::search(workspace, path, query, *limit, budget)
            }
        };
        match result {
            Ok(output) => output,
            Err(error) => Output::error(&error.to_string()),
        }
    }
}
fn string(text: &str) -> Value {
    Value::String(text.into())
}
fn number(value: usize) -> Value {
    Value::Number(value.to_string())
}
