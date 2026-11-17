use super::jsonl;
use crate::{agent::Agent, attachments::Prompt, console::Console, json::Value};
use std::io::Write;

/// Complete the one-shot lifecycle before publishing its final result.
pub(super) fn run(
    agent: &mut Agent,
    prompt: Prompt,
    json: bool,
    output: &mut impl Write,
    status: &mut impl Write,
) -> Result<(), String> {
    if !json {
        return agent.run_single_turn(prompt, &mut Console::new(output, status));
    }
    let mut lines = jsonl::Lines::new(output, agent.redactor());
    let result = agent.run_single_turn(prompt, &mut lines);
    let mut fields = vec![
        ("type", Value::string("result")),
        (
            "status",
            Value::string(if result.is_ok() {
                "completed"
            } else {
                "failed"
            }),
        ),
        (
            "session_id",
            Value::optional(agent.sessions().map(|handle| handle.id())),
        ),
        ("provider", Value::string(agent.backend_id())),
        ("model", Value::string(agent.model())),
        ("effort", Value::string(agent.effort().name())),
        (
            "permissions",
            Value::string(agent.permissions().level().id()),
        ),
    ];
    if let Err(error) = &result {
        fields.push(("error", Value::string(agent.redact(error))));
    }
    lines.write(Value::entries(fields))?;
    result
}

#[cfg(test)]
mod tests;
