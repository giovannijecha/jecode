use crate::agent::Agent;
use std::io::Write;

/// `/checkpoints ...` in plain mode.
pub(super) fn print(agent: &Agent, arguments: &str, output: &mut impl Write) -> Result<(), String> {
    let command = format!("/checkpoints {arguments}").trim_end().to_string();
    let result = agent.checkpoints_command(arguments)?;
    // A recorded file is shown, not copied into the conversation.
    let details = if result.raw.is_some() {
        &[][..]
    } else {
        &result.details
    };
    agent.record_local_details(&command, &result.title, "notice", details);
    writeln!(output, "{}", agent.redact(&result.text())).map_err(|error| error.to_string())
}
