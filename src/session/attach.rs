//! `/attach` in the line-based chat: files or saved references for the next message.

use crate::{
    agent::Agent,
    attachments::{Attachment, paths},
    sessions::Handle,
};
use std::collections::BTreeMap;
use std::io::Write;

/// The path list of an `/attach` line.
pub fn command(line: &str) -> Option<&str> {
    let (name, arguments) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    name.eq_ignore_ascii_case("/attach").then_some(arguments)
}

pub fn stage(
    agent: &Agent,
    arguments: &str,
    staged: &mut Vec<Attachment>,
    output: &mut impl Write,
) -> Result<(), String> {
    if arguments.trim() == "--clear" {
        staged.clear();
        save(agent, staged)?;
        return writeln!(output, "Staged attachments cleared.").map_err(|error| error.to_string());
    }
    let directory = std::env::current_dir().map_err(|error| error.to_string())?;
    let imported = if arguments.contains("attachment:") {
        let pool = agent
            .sessions()
            .ok_or("Attachment storage is unavailable")?
            .store()
            .attachments();
        paths::words(arguments)?
            .iter()
            .map(|word| {
                if let Some(id) = word.strip_prefix("attachment:") {
                    pool.load(id).map(|stored| stored.attachment)
                } else {
                    pool.import_file(
                        &paths::argument(word, &directory),
                        &crate::cancel::Cancellation::default(),
                    )
                }
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        agent.import_attachments(&paths::typed(arguments, &directory)?)?
    };
    staged.extend(imported);
    save(agent, staged)?;
    announce(staged, output)
}

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
