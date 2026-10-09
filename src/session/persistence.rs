use super::{SessionConfig, delete};
use crate::{agent::Agent, input::read_line, sessions::Record};
use std::io::{BufRead, Write};

pub(super) fn hint(agent: &Agent, output: &mut impl Write) -> Result<(), String> {
    let handle = agent.sessions().ok_or("Session storage is unavailable")?;
    let listing = handle.store().list()?;
    if let Some(session) = listing
        .sessions
        .iter()
        .find(|session| session.id != handle.id())
    {
        writeln!(output, "Previous session: {} · /resume", session.title)
            .map_err(|error| error.to_string())?;
    }
    for warning in listing.warnings {
        writeln!(output, "{warning}").map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(super) fn resume(
    agent: &mut Agent,
    config: &SessionConfig,
    id: Option<&str>,
    input: &mut impl BufRead,
    output: &mut impl Write,
    status: &mut impl Write,
) -> Result<(), String> {
    let id = if let Some(id) = id {
        id.to_string()
    } else {
        'listing: loop {
            let (sessions, warnings) = delete::sessions(agent)?;
            for warning in warnings {
                writeln!(status, "{}", agent.redact(&warning))
                    .map_err(|error| error.to_string())?;
            }
            let current = agent
                .sessions()
                .ok_or("Session storage is unavailable")?
                .id();
            writeln!(output, "Resume · current folder").map_err(|error| error.to_string())?;
            if sessions.is_empty() {
                writeln!(output, "No saved conversations.").map_err(|error| error.to_string())?;
            }
            for (index, session) in sessions.iter().enumerate() {
                writeln!(
                    output,
                    "{}. {} · {}{}",
                    index + 1,
                    session.title,
                    session.description(),
                    if session.id == current {
                        " · current"
                    } else {
                        ""
                    }
                )
                .map_err(|error| error.to_string())?;
            }
            loop {
                write!(output, "{}", if sessions.is_empty() {
                    "Press Enter or /cancel to close: "
                } else {
                    "Choose a number or session ID to resume, d NUMBER or d ID to delete (/cancel closes): "
                })
                    .and_then(|_| output.flush())
                    .map_err(|error| error.to_string())?;
                let Some(value) = read_line(input)? else {
                    return Ok(());
                };
                let value = value.trim();
                if value == "/cancel" || value.is_empty() {
                    return Ok(());
                }
                let (deleting, choice) = match value.strip_prefix("d ") {
                    Some(choice) => (true, choice.trim()),
                    None => (false, value),
                };
                let session = choice
                    .parse::<usize>()
                    .ok()
                    .and_then(|number| number.checked_sub(1))
                    .and_then(|index| sessions.get(index))
                    .or_else(|| sessions.iter().find(|session| session.id == choice));
                if let Some(session) = session {
                    if deleting {
                        delete::confirm(agent, config, session, input, output)?;
                        continue 'listing;
                    }
                    if session.id == current {
                        writeln!(output, "Current conversation kept.")
                            .map_err(|error| error.to_string())?;
                        return Ok(());
                    }
                    break 'listing session.id.clone();
                }
                writeln!(output, "Choose a listed session or /cancel.")
                    .map_err(|error| error.to_string())?;
            }
        }
    };
    let result = agent.resume(&id)?;
    agent.record_local(&format!("/resume {id}"), &result);
    agent.save_session()?;
    let document = agent.sessions().unwrap().snapshot();
    for record in document.records() {
        match record {
            Record::Text { role, text } => {
                if role == "user" {
                    writeln!(output, "\n> {text}")
                } else {
                    writeln!(output, "\n{text}")
                }
                .map_err(|error| error.to_string())?;
            }
            Record::Tool {
                name,
                arguments,
                summary,
                ..
            } => {
                writeln!(
                    status,
                    "[restored tool: {name}] {} · {summary}",
                    crate::events::description(&name, &arguments)
                )
                .map_err(|error| error.to_string())?;
            }
            Record::Local {
                command,
                result,
                details,
                ..
            } => {
                writeln!(output, "\n{command}\n  {result}").map_err(|error| error.to_string())?;
                for (key, value) in details {
                    writeln!(output, "  {key}  {value}").map_err(|error| error.to_string())?;
                }
            }
        }
    }
    if !document.input.draft.text.is_empty() {
        writeln!(
            output,
            "\nRecovered draft (not sent; copy/edit before sending):\n{}",
            document.input.draft.prompt().display()
        )
        .map_err(|error| error.to_string())?;
        recover_attachments(&document.input.draft, output)?;
    }
    for (index, draft) in document.input.paused.iter().enumerate() {
        writeln!(
            output,
            "\nPaused draft {}/{} (not sent; copy/edit before sending):\n{}",
            index + 1,
            document.input.paused.len(),
            draft.prompt().display()
        )
        .map_err(|error| error.to_string())?;
        recover_attachments(draft, output)?;
    }
    Ok(())
}

fn recover_attachments(
    draft: &crate::sessions::Draft,
    output: &mut impl Write,
) -> Result<(), String> {
    if !draft.attachments.is_empty() {
        let references = draft
            .attachments
            .iter()
            .map(crate::attachments::Attachment::reference)
            .collect::<Vec<_>>()
            .join(" ");
        writeln!(
            output,
            "Restore these attachments before sending: /attach {references}"
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}
