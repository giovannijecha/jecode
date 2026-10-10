use super::Agent;
use crate::json::Value;

const VERIFICATION_GUIDANCE: &str = "\
Keep the working directory focused on project deliverables: source, useful durable tests and fixtures, configuration and build recipes. Follow the project's testing conventions; add lasting test infrastructure or dependencies only when justified by the project's needs. Run relevant checks and stop expanding verification once they pass unless a concrete concern remains.

Use the session temporary area for one-off verification scripts, check-only fixtures, experiments, tooling, configuration and dependencies needed only for a temporary check. Put generated screenshots, traces, logs, reports, dumps and browser profiles there unless the user or project requires them as deliverables. Set explicit output paths under the quoted $JECODE_TMP path; temporary environment variables do not redirect project-relative writes.

Before finishing, review your changes and move only disposable artifacts you created into the session temporary area. Preserve existing user files and artifacts, useful durable tests and required project outputs.";

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
                    "You are Jecode, a personal terminal coding agent.\n\nWorking files and verification:\n{VERIFICATION_GUIDANCE}\n\nEnvironment:\n{}",
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
