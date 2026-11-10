use super::Agent;
use crate::backend::{CacheRetention, retention};
use crate::context::SNAPSHOT_HEADER;
use crate::json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const PROMPT: &str = include_str!("PROMPT.md");
pub(super) const NO_INSTRUCTIONS: &str =
    "Project instructions from AGENTS.md: none apply now; earlier ones no longer apply.";
const NO_SKILLS: &str =
    "Personal skills: none are available now; earlier catalog entries no longer apply.";

/// Request instructions, collaborator reports and finished processes, read
/// before `messages` is locked.
pub(super) struct Environment(
    String,
    pub(super) Vec<(String, String)>,
    pub(super) Vec<crate::tools::processes::Completion>,
);

fn join<const N: usize>(parts: [String; N]) -> String {
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

impl Agent {
    pub(super) fn system_message(&self) -> Value {
        let environment = Value::object([
            (
                "working_directory",
                Value::string(self.tools.root().to_string_lossy()),
            ),
            ("host_operating_system", Value::string(std::env::consts::OS)),
            ("command_shell", Value::string("Jecode shell")),
            (
                "file_tool_access",
                Value::string("project_directory_and_session_and_skill_references"),
            ),
            (
                "bash_access",
                Value::string("Workspace or Full permission level"),
            ),
            ("tool_execution", Value::string("direct")),
            ("background_resume", Value::string(self.background_resume())),
            // Spares version-control probes in folders that have none.
            (
                "git_repository",
                Value::Bool(
                    self.tools
                        .root()
                        .ancestors()
                        .any(|folder| folder.join(".git").exists()),
                ),
            ),
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

    /// The native prompt and environment. Project instructions and the skill
    /// catalog change during a conversation, so they travel in an instruction
    /// snapshot instead and leave this request prefix unchanged.
    pub(super) fn current_system_message(&self) -> Value {
        let mut message = self.system_message();
        // The prompt opens every conversation's instructions; adapters with
        // explicit cache breakpoints keep it apart from the folder's part.
        if let Value::Object(fields) = &mut message {
            fields.insert(
                "jecode_cache_prefix".into(),
                Value::number(PROMPT.trim().len()),
            );
            fields.insert(
                retention::MARKER.into(),
                Value::string(self.cache_retention().id()),
            );
        }
        message
    }

    /// The complete current instructions. An empty section says so explicitly,
    /// so a snapshot withdraws earlier rules rather than omitting them.
    pub(super) fn instruction_snapshot(&self) -> String {
        let instructions = if self.project_instructions.is_empty() {
            NO_INSTRUCTIONS.to_owned()
        } else {
            format!(
                "Project instructions from AGENTS.md:\n\n{}",
                self.redact(&self.project_instructions)
            )
        };
        let skills = self.skill_instructions();
        let skills = if skills.is_empty() {
            NO_SKILLS.to_owned()
        } else {
            self.redact(&skills)
        };
        format!("{SNAPSHOT_HEADER}\n\n{instructions}\n\n{skills}")
    }

    /// Whether any project instructions or catalog entries currently apply.
    pub(super) fn has_instructions(&self) -> bool {
        !self.project_instructions.is_empty() || !self.skill_instructions().is_empty()
    }

    /// A user preference read at every request, like auto-compact.
    pub fn set_cache_retention(&self, retention: CacheRetention) {
        self.cache_retention
            .store(retention == CacheRetention::Long, Ordering::Relaxed);
    }

    pub fn cache_retention(&self) -> CacheRetention {
        if self.cache_retention.load(Ordering::Relaxed) {
            CacheRetention::Long
        } else {
            CacheRetention::Short
        }
    }

    pub(crate) fn cache_retention_preference(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cache_retention)
    }

    #[cfg(test)]
    pub(super) fn request_environment(&self) -> String {
        let environment = self.environment();
        let messages = self.messages.lock().unwrap();
        self.request_environment_for(&environment, &messages)
    }

    /// Reads the request instructions that do not depend on messages. Call it
    /// before locking `messages`: a collaborator receipt holds collaboration
    /// state while it appends to this conversation, so the lock order is
    /// collaboration state, then messages. One snapshot serves the instructions
    /// and the reports, so they describe the same moment.
    pub(super) fn environment(&self) -> Environment {
        let records = if self.child {
            Vec::new()
        } else {
            self.collaboration.snapshot()
        };
        Environment(
            join([
                self.permission_instructions(),
                self.temporary_instructions(),
                collaboration_instructions(&records),
            ]),
            collaboration_reports(&records),
            self.tools.process_completions(&self.redactor),
        )
    }

    pub(super) fn request_environment_for(
        &self,
        environment: &Environment,
        messages: &[Value],
    ) -> String {
        join([environment.0.clone(), self.todo_instructions(messages)])
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
}

fn collaboration_instructions(records: &[Value]) -> String {
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
        // Only transitions change this line: state persists at the position
        // first sent, so progress text would grow every request. Reports
        // join once through `collaboration_reports`.
        lines.push(format!(
            "{id} {title}: {state}, received assignment v{received}, report {}, {}",
            report_version.map_or_else(|| "none".to_owned(), |version| format!("v{version}")),
            review_state
        ));
    }
    lines.join("\n")
}

/// Reports of active collaborators as `(marker, entry)`. Runtime state
/// carries each report once, where it arrived, rather than on every update.
fn collaboration_reports(records: &[Value]) -> Vec<(String, String)> {
    records
        .iter()
        .filter_map(|record| {
            let id = record.get("id").and_then(Value::as_str)?;
            let report = record.get("report")?;
            let version = report.get("version").and_then(Value::as_usize)?;
            let text = report.get("text").and_then(Value::as_str)?;
            let marker = format!("{id} report v{version}:");
            let entry = format!("{marker} {}", text.chars().take(2000).collect::<String>());
            Some((marker, entry))
        })
        .collect()
}
