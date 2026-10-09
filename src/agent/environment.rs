use super::Agent;
use crate::json::Value;

impl Agent {
    pub(super) fn system_message(&self) -> Value {
        let environment = Value::object([
            (
                "working_directory",
                Value::string(self.tools.root().to_string_lossy()),
            ),
            (
                "file_tool_access",
                Value::string("project_directory_and_session_references"),
            ),
            ("bash_access", Value::string("normal_system_access")),
            ("tool_execution", Value::string("direct")),
        ]);
        Value::object([
            ("role", Value::string("system")),
            (
                "content",
                Value::string(format!(
                    "You are Jecode, a personal terminal coding agent.\n\nEnvironment:\n{}",
                    environment.encode()
                )),
            ),
        ])
    }

    pub(super) fn current_system_message(&self) -> Value {
        let mut message = self.system_message();
        if !self.project_instructions.is_empty()
            && let Value::Object(fields) = &mut message
        {
            let native = fields.get("content").and_then(Value::as_str).unwrap_or("");
            let content = format!(
                "{native}\n\nProject instructions from JECODE.md:\nApply these instructions to work in this project. Explicit user requests take precedence; Jecode's tool contracts still apply.\n\n{}",
                self.redact(&self.project_instructions)
            );
            fields.insert("content".into(), Value::string(content));
        }
        message
    }

    pub(super) fn request_environment(&self, messages: &[Value]) -> String {
        let mut environment = self.temporary_instructions();
        if let Some(request) = messages
            .iter()
            .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        {
            let state = Value::object([(
                "latest_user_request",
                Value::string(format!("history:{request}")),
            )]);
            environment.push_str(&format!("\n\nNative request state:\n{}", state.encode()));
        }
        let review = self.completion_review_state(messages);
        if !review.is_empty() {
            environment.push_str("\n\n");
            environment.push_str(&review);
        }
        environment
    }

    pub(super) fn completion_review_state(&self, messages: &[Value]) -> String {
        if !self.reviewing_completion {
            return String::new();
        }
        let candidate = messages.iter().rposition(|message| {
            message.get("role").and_then(Value::as_str) == Some("assistant")
                && message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .is_none_or(|calls| calls.is_empty())
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.trim().is_empty())
        });
        let state = Value::object([
            ("status", Value::string("reviewing")),
            (
                "candidate_response",
                candidate.map_or(Value::Null, |at| Value::string(format!("history:{at}"))),
            ),
            ("candidate_delivered", Value::Bool(false)),
            (
                "native_blocker",
                self.context
                    .evidence
                    .protection_problem()
                    .map_or(Value::Null, Value::string),
            ),
        ]);
        format!("Native completion state:\n{}", state.encode())
    }
}

#[cfg(test)]
mod tests;
