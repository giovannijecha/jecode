use super::{Context, memory};
use crate::json::{self, Value};

impl Context {
    /// Native boundaries and recorded results surround, but do not endorse, semantic memory.
    pub(super) fn state_view(&self, messages: &[Value]) -> String {
        let reference = |at| Value::string(format!("history:{at}"));
        let latest = messages
            .iter()
            .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"));
        let proposed = json::parse(&self.summary).ok();
        let reviewed = proposed
            .as_ref()
            .and_then(|value| value.get("reviewed_request_history"))
            .and_then(Value::as_usize)
            .filter(|at| {
                messages
                    .get(*at)
                    .and_then(|message| message.get("role"))
                    .and_then(Value::as_str)
                    == Some("user")
            });
        let mut facts = self.evidence.prompt_value();
        if let Value::Object(fields) = &mut facts {
            for key in [
                "last_tool",
                "last_tracked_change",
                "checks",
                "observed_indirect_changes",
            ] {
                match fields.get_mut(key) {
                    Some(Value::Array(entries)) => {
                        for entry in entries {
                            provenance(entry, messages);
                        }
                    }
                    Some(entry) => provenance(entry, messages),
                    None => {}
                }
            }
        }
        let state = Value::object([
            ("latest_user_request", latest.map_or(Value::Null, reference)),
            (
                "memory_reviewed_request",
                reviewed.map_or(Value::Null, reference),
            ),
            ("live_transcript_from", Value::number(self.from)),
            ("original_message_count", Value::number(messages.len())),
            ("original_requests", Value::string("history:requests")),
            ("complete_memory", Value::string("history:memory")),
            (
                "memory_source",
                Value::string("model_proposal_with_native_reference_and_retention_validation"),
            ),
            ("memory_proves_execution", Value::Bool(false)),
            (
                "memory",
                json::parse(&self.memory_view())
                    .unwrap_or_else(|_| Value::string(self.memory_view())),
            ),
            ("execution_facts", facts),
            ("file_tracking_scope", Value::string("observed_files_only")),
            (
                "passed_check_scope",
                Value::string("successful_command_exit; requirement_coverage_not_established"),
            ),
        ]);
        format!("Native context state:\n{}", state.encode())
    }
}

fn provenance(entry: &mut Value, messages: &[Value]) {
    let Some(at) = entry.get("message").and_then(Value::as_usize) else {
        return;
    };
    if let Value::Object(fields) = entry {
        fields.insert("source".into(), memory::source_record(messages, at));
    }
}

#[cfg(test)]
mod tests;
