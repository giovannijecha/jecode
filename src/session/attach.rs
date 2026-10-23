//! Attachment staging for the line-based chat and saved sessions.

use crate::{agent::Agent, attachments::Attachment, sessions::Handle};
use std::collections::BTreeMap;
use std::io::Write;

/// Keep plain-chat pending attachments in the session without changing its
/// composer draft, queue or history.
pub fn save(agent: &Agent, staged: &[Attachment]) -> Result<(), String> {
    let handle = agent.sessions().ok_or("Session storage is unavailable")?;
    let mut input = handle.snapshot().input;
    input.staged = staged.to_vec();
    handle.input(input);
    handle.flush()
}

/// Move the active staging list to a resumed session before releasing its
/// references in the previous session. Existing staged entries at the target
/// are retained, including repeated attachments.
pub fn transfer(
    agent: &Agent,
    previous: Option<&Handle>,
    staged: &mut Vec<Attachment>,
) -> Result<(), String> {
    let current = agent.sessions().ok_or("Session storage is unavailable")?;
    let saved = current.snapshot().input.staged;
    let mut present = BTreeMap::<&str, usize>::new();
    for attachment in staged.iter() {
        *present.entry(&attachment.id).or_default() += 1;
    }
    let mut target_counts = BTreeMap::<&str, usize>::new();
    let mut additional = Vec::new();
    for attachment in &saved {
        let count = target_counts.entry(&attachment.id).or_default();
        *count += 1;
        if *count > present.get(attachment.id.as_str()).copied().unwrap_or(0) {
            additional.push(attachment.clone());
        }
    }
    staged.extend(additional);
    save(agent, staged)?;
    if let Some(previous) = previous
        && previous.id() != current.id()
    {
        let mut input = previous.snapshot().input;
        if !input.staged.is_empty() {
            input.staged.clear();
            previous.input(input);
            previous.flush()?;
        }
    }
    Ok(())
}

pub fn announce(staged: &[Attachment], output: &mut impl Write) -> Result<(), String> {
    let labels = staged
        .iter()
        .enumerate()
        .map(|(index, attachment)| attachment.label(index + 1))
        .collect::<Vec<_>>()
        .join(" ");
    writeln!(
        output,
        "Staged {labels} for your next message. Press Enter alone to send only the attachments."
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
