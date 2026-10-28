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

    #[cfg(test)]
    pub(super) fn request_environment(&self) -> String {
        let messages = self.messages.lock().unwrap();
        self.request_environment_for(&messages)
    }

    pub(super) fn request_environment_for(&self, messages: &[Value]) -> String {
        [
            self.temporary_instructions(),
            self.checkpoint_instructions(),
            self.collaboration_instructions(),
            self.todo_instructions(messages),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
    }

    fn todo_instructions(&self, messages: &[Value]) -> String {
        let Some(plan) = crate::todo::latest(messages) else {
            return String::new();
        };
        let mut text = format!(
            "Current conversation plan, revision {}: {}. The model owns updates through todo_list action=set; action=get retrieves the full plan.\n",
            plan.revision, plan.title
        );
        for task in &plan.tasks {
            let row = format!(
                "{} [{}] {}{}\n",
                task.id,
                task.status,
                task.title,
                task.agent
                    .as_ref()
                    .map_or(String::new(), |agent| format!(" ({agent})"))
            );
            if text.len() + row.len() > 4_000 {
                text.push_str(
                    "Further tasks omitted here; call todo_list action=get for the full plan.\n",
                );
                break;
            }
            text.push_str(&row);
        }
        text
    }

    fn collaboration_instructions(&self) -> String {
        if self.child {
            return String::new();
        }
        let records = self.collaboration.snapshot();
        if records.is_empty() {
            return String::new();
        }
        let mut lines = vec![
            "Active collaborators in this session. Only you, the main agent, review their work. For each returned or failed report still awaiting review, inspect and verify the evidence, then call agent with action=review, id, report_version from the report, and a note recording your assessment. Inspection or a text summary does not record review. After any needed follow-up or integration, call agent with action=close to release the collaborator and remove it from the active list.".to_owned(),
        ];
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
            let reviewed = record.get("reviewed") == Some(&Value::Bool(true));
            let report_version = record
                .get("report")
                .and_then(|report| report.get("version"))
                .and_then(Value::as_usize);
            let review_state = match (state.as_str(), reviewed) {
                ("returned" | "failed", false) => "awaiting main-agent review",
                ("returned" | "failed", true) => "reviewed; close when finished",
                ("running" | "stopping", _) => "working",
                _ => "close when safe to release",
            };
            lines.push(format!(
                "{id} {title}: {state}, received assignment v{received}, report {}, {}",
                report_version.map_or_else(|| "none".to_owned(), |version| format!("v{version}")),
                review_state
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
