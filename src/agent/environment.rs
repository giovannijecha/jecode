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
            ("host_operating_system", Value::string(std::env::consts::OS)),
            ("command_shell", Value::string("Bash")),
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
                "{native}\n\nProject instructions from AGENTS.md:\nEach source below applies only to its stated directory scope. More specific directory instructions take precedence on conflicts. Explicit user requests take precedence over project instructions; Jecode's tool contracts still apply.\n\n{}",
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
            self.collaboration_instructions(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
    }

    fn collaboration_instructions(&self) -> String {
        if self.child {
            return String::new();
        }
        let records = self.collaboration.snapshot();
        if records.is_empty() {
            return String::new();
        }
        let mut lines = vec!["Collaborators in this session (inspect with the agent tool; returned work still needs your review):".to_owned()];
        for record in records {
            let field = |name| {
                record
                    .get(name)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned()
            };
            let id = field("id");
            let title = field("title");
            let state = field("state");
            let received = record
                .get("received_version")
                .and_then(Value::as_usize)
                .unwrap_or(0);
            lines.push(format!(
                "{id} {title}: {state}, received assignment v{received}"
            ));
            if let Some(report) = record
                .get("report")
                .and_then(|report| report.get("text"))
                .and_then(Value::as_str)
            {
                lines.push(format!(
                    "{id} report: {}",
                    report.chars().take(2000).collect::<String>()
                ));
            } else if let Some(update) = record
                .get("updates")
                .and_then(Value::as_array)
                .and_then(|updates| updates.last())
                .and_then(Value::as_str)
            {
                lines.push(format!(
                    "{id} latest update: {}",
                    update.chars().take(1000).collect::<String>()
                ));
            }
        }
        lines.join("\n")
    }
}
