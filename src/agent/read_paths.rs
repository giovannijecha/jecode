//! One read call for several paths. Models tend to send one call per
//! response, and every response sends the conversation again; reading the
//! files a step needs together saves those round trips.

use super::Agent;
use crate::{backend::ToolCall, json::Value};

/// Paths one call may read.
pub(crate) const MAX_PATHS: usize = 12;
/// Combined result size; later paths report that they were not read.
const MAX_BYTES: usize = 96 * 1024;

impl Agent {
    /// Reads each path with its default window, in order, through the same
    /// route as a single read. Every entry reports its own result or error,
    /// so one missing file needs no retry of the others.
    pub(super) fn read_paths(&self, call: &ToolCall, arguments: &Value) -> Value {
        let Some(paths) = arguments.get("paths").and_then(Value::as_array) else {
            return failure("paths must be an array of strings");
        };
        if arguments.get("path").is_some() {
            return failure("Use path or paths, not both");
        }
        if paths.is_empty() || paths.len() > MAX_PATHS {
            return failure(&format!("paths must contain 1-{MAX_PATHS} entries"));
        }
        let mut used = 0usize;
        let files = paths
            .iter()
            .map(|path| {
                let Some(path) = path.as_str() else {
                    return failure("paths must contain strings");
                };
                let single = Value::object([("path", Value::string(path))]);
                let result = if used >= MAX_BYTES {
                    failure("Not read: this call reached its combined size; read it with path")
                } else if self
                    .tools
                    .access("read", &single.encode())
                    .is_some_and(|access| access.outside)
                {
                    failure("Outside the working folder; read it alone with path to request access")
                } else {
                    let read = ToolCall {
                        id: call.id.clone(),
                        name: "read".into(),
                        arguments: single.encode(),
                    };
                    shown_alone(
                        self.execute_tool_with_change(&read, crate::tools::Begun::default())
                            .result,
                    )
                };
                used = used.saturating_add(result.encode().len());
                entry(path, result)
            })
            .collect();
        Value::object([("files", Value::Array(files))])
    }
}

fn failure(message: &str) -> Value {
    Value::object([("error", Value::string(message))])
}

/// Media are shown to the model only after a single read; other attachments
/// return their text page or path in this call.
fn shown_alone(result: Value) -> Value {
    if result.get("view").is_some() {
        return failure("Media file: read it alone with path to view it");
    }
    result
}

/// The result named by the path the model asked for.
fn entry(path: &str, result: Value) -> Value {
    let Value::Object(mut fields) = result else {
        return result;
    };
    fields.insert("path".into(), Value::string(path));
    Value::Object(fields)
}

#[cfg(test)]
mod tests;
