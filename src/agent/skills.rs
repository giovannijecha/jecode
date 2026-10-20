use super::Agent;
use crate::json::{self, Value};

impl Agent {
    /// Refreshes the user library for the local /skills listing.
    pub fn skills(&mut self) -> Result<Vec<crate::skills::SkillInfo>, String> {
        self.refresh_skills();
        Ok(self.skills.list())
    }

    pub fn skill_warnings(&self) -> &[String] {
        self.skills.warnings()
    }

    pub(super) fn refresh_skills(&mut self) {
        if self.skills.refresh() {
            self.context.input_tokens = None;
            self.context.measured_end = 0;
        }
    }

    pub(super) fn skill_instructions(&self) -> String {
        let used = self.used_skills.iter().cloned().collect::<Vec<_>>();
        self.skills.instructions(&used)
    }

    pub(super) fn rebuild_used_skills(&mut self) {
        // Original messages survive native compaction and session resume. A
        // successful read receipt provides a durable reference without storing
        // the entire skill body in the dynamic system prompt.
        let messages = self.messages.lock().unwrap();
        self.used_skills.clear();
        for message in messages.iter() {
            if message.get("role").and_then(Value::as_str) != Some("tool") {
                continue;
            }
            let Some(reference) = message
                .get("content")
                .and_then(Value::as_str)
                .and_then(|content| json::parse(content).ok())
                .and_then(|receipt| {
                    receipt
                        .get("skill_reference")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
            else {
                continue;
            };
            if let Ok((name, _)) = crate::skills::parse_reference(&reference) {
                self.used_skills.insert(name.to_owned());
            }
        }
    }
}
