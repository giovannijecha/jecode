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

    pub(super) fn request_environment(&self) -> String {
        self.temporary_instructions()
    }
}
