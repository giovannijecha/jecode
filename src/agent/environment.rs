use super::Agent;
use crate::json::Value;

const PROMPT: &str = include_str!("PROMPT.md");

impl Agent {
    pub(super) fn system_message(&self) -> Value {
        let environment = Value::object([
            (
                "working_directory",
                Value::string(self.tools.root().to_string_lossy()),
            ),
            (
                "file_tool_access",
                Value::string("project_directory_and_session_and_skill_references"),
            ),
            ("bash_access", Value::string("normal_system_access")),
            ("tool_execution", Value::string("direct")),
        ]);
        Value::object([
            ("role", Value::string("system")),
            (
                "content",
                Value::string(format!(
                    "{}\n\nEnvironment:\n{}",
                    PROMPT.trim(),
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
                "{native}\n\nProject instructions from JECODE.md:\nEach source below applies only to its stated directory scope. More specific directory instructions take precedence on conflicts. Explicit user requests take precedence over project instructions; Jecode's tool contracts still apply.\n\n{}",
                self.redact(&self.project_instructions)
            );
            fields.insert("content".into(), Value::string(content));
        }
        let skills = self.skill_instructions();
        if !skills.is_empty()
            && let Value::Object(fields) = &mut message
        {
            let native = fields.get("content").and_then(Value::as_str).unwrap_or("");
            let content = format!("{native}\n\n{}", self.redact(&skills));
            fields.insert("content".into(), Value::string(content));
        }
        message
    }

    pub(super) fn request_environment(&self) -> String {
        [
            self.temporary_instructions(),
            self.checkpoint_instructions(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
    }
}
