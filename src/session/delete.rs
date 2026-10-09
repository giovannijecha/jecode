use super::SessionConfig;
use crate::{
    agent::Agent,
    input::read_line,
    sessions::{DeleteReport, Summary},
};
use std::io::{BufRead, Write};

pub(crate) fn sessions(agent: &Agent) -> Result<(Vec<Summary>, Vec<String>), String> {
    let handle = agent.sessions().ok_or("Session storage is unavailable")?;
    let mut listing = handle.store().list()?;
    let document = handle.snapshot();
    if !document.meaningful() {
        listing.sessions.retain(|entry| entry.id != document.id);
    } else if !listing.sessions.iter().any(|entry| entry.id == document.id) {
        listing.sessions.insert(
            0,
            Summary {
                id: document.id.clone(),
                title: document.title(),
                updated: document.updated,
                model: document.model,
            },
        );
    }
    Ok((listing.sessions, listing.warnings))
}

pub(crate) fn message(title: &str, report: &DeleteReport, current: bool) -> String {
    let subject = if current {
        "Conversation deleted".into()
    } else {
        format!("Deleted {title}")
    };
    let mut text = format!(
        "{subject} · {} files and {} directories removed",
        report.files, report.directories
    );
    if current {
        text.push_str(" · new conversation opened");
    }
    if report.legacy_references > 0 {
        text.push_str(&format!(
            " · {} older shared output references kept",
            report.legacy_references
        ));
    }
    text
}

pub(super) fn confirm(
    agent: &mut Agent,
    config: &SessionConfig,
    session: &Summary,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<Option<bool>, String> {
    let id = &session.id;
    writeln!(output, "Delete {} · {id}?\nConversation, owned tool outputs and temporary files will be removed.\nIf current, a new conversation opens and unsent input is kept.", session.title).map_err(|e| e.to_string())?;
    write!(output, "Type delete to confirm [Enter = cancel]: ")
        .and_then(|()| output.flush())
        .map_err(|e| e.to_string())?;
    let Some(value) = read_line(input)? else {
        return Ok(None);
    };
    if value.trim() != "delete" {
        return Ok(None);
    }
    let (report, current) =
        agent.delete_session(id, config.settings.model.clone(), config.settings.effort)?;
    let text = message(&session.title, &report, current);
    agent.record_local_details(
        &format!("/resume · delete {id}"),
        &text,
        if report.legacy_references > 0 {
            "warning"
        } else {
            "notice"
        },
        &[],
    );
    writeln!(output, "{}", agent.redact(&text)).map_err(|e| e.to_string())?;
    Ok(Some(current))
}

#[cfg(test)]
mod tests;
