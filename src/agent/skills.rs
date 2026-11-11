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

    /// Where the user library lives, so /skills can list it during a turn.
    pub fn skill_directory(&self) -> Option<std::path::PathBuf> {
        self.skills.directory().map(Into::into)
    }

    pub(super) fn refresh_skills(&mut self) {
        if self.skills.refresh() {
            self.context.clear_measurement();
        }
    }

    pub(super) fn skill_instructions(&self) -> String {
        self.skills.instructions()
    }

    pub(super) fn skill_checkpoint_instructions(&self) -> String {
        let used = self.used_skills.iter().cloned().collect::<Vec<_>>();
        self.skills.checkpoint_instructions(&used)
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
            let Some(receipt) = message
                .get("content")
                .and_then(Value::as_str)
                .and_then(|content| json::parse(content).ok())
            else {
                continue;
            };
            self.used_skills
                .extend(delivered(&receipt).map(str::to_owned));
        }
    }
}

/// The skills a read result delivered: its own, each file's of a multi-path
/// read, or those of the reads in a `steps` result.
pub(super) fn delivered(result: &Value) -> impl Iterator<Item = &str> {
    let steps = result
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("result"));
    std::iter::once(result)
        .chain(steps)
        .flat_map(|item| {
            let files = item.get("files").and_then(Value::as_array);
            std::iter::once(item).chain(files.into_iter().flatten())
        })
        .flat_map(|item| {
            let single = item.get("skill_reference").and_then(Value::as_str);
            let saved = item
                .get("skill_references")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str);
            single.into_iter().chain(saved)
        })
        .filter_map(|reference| crate::skills::parse_reference(reference).ok())
        .map(|(name, _)| name)
}
