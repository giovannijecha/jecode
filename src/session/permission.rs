//! The line-based permission request: the call, the access and three
//! numbered choices. Closed input denies, so nothing runs unattended.

use crate::agent::Agent;
use crate::input::read_line;
use crate::permissions::{Decision, Level, Permissions, Request, wording};
use std::io::{BufRead, Write};

const USAGE: &str = "Usage: /permissions [read|workspace|full|remembered|revoke N]";

/// The line-based counterpart to the conversation level and remembered menu.
pub(super) fn command(
    agent: &Agent,
    arguments: &str,
    output: &mut impl Write,
) -> Result<(), String> {
    let permissions = agent.permissions();
    let arguments = arguments.trim();
    if arguments.is_empty() {
        let current = permissions.level();
        writeln!(output, "Permissions · this conversation").map_err(|error| error.to_string())?;
        for level in Level::ALL {
            let marker = if level == current { " (current)" } else { "" };
            writeln!(output, "  {}{marker} · {}", level.name(), level.fact())
                .map_err(|error| error.to_string())?;
        }
        let grants = permissions.grants().len();
        writeln!(
            output,
            "Default for new conversations: {} · {} remembered grant{}{}\nChoose with /permissions read|workspace|full; inspect with /permissions remembered.",
            permissions.default_level().name(),
            grants,
            if grants == 1 { "" } else { "s" },
            if grants > 0 && current != Level::Workspace {
                " (paused at this level)"
            } else {
                ""
            },
        )
        .map_err(|error| error.to_string())?;
        return Ok(());
    }
    if let Some(level) = Level::parse(&arguments.to_ascii_lowercase()) {
        let previous = permissions.level();
        permissions.set_level(level)?;
        return writeln!(
            output,
            "Permissions: {} → {} · this conversation",
            previous.name(),
            level.name()
        )
        .map_err(|error| error.to_string());
    }
    if arguments.eq_ignore_ascii_case("remembered") {
        let grants = permissions.grants();
        if grants.is_empty() {
            return writeln!(output, "Nothing remembered for this conversation.")
                .map_err(|error| error.to_string());
        }
        writeln!(output, "Remembered grants · this conversation")
            .map_err(|error| error.to_string())?;
        for (index, grant) in grants.iter().enumerate() {
            let place = grant
                .place
                .as_ref()
                .map_or(String::new(), |place| format!(" in {place}"));
            let row = format!(
                "  {} {} · {}{place}",
                index + 1,
                grant.kind.name(),
                grant.target
            );
            writeln!(output, "{}", agent.redact(&row)).map_err(|error| error.to_string())?;
        }
        if permissions.level() != Level::Workspace {
            writeln!(output, "Paused at {}.", permissions.level().name())
                .map_err(|error| error.to_string())?;
        }
        return writeln!(output, "Revoke with /permissions revoke N.")
            .map_err(|error| error.to_string());
    }
    if let Some((action, index)) = arguments.split_once(char::is_whitespace)
        && action.eq_ignore_ascii_case("revoke")
    {
        let index = index
            .trim()
            .parse::<usize>()
            .map_err(|_| USAGE.to_owned())?;
        let grants = permissions.grants();
        let grant = index
            .checked_sub(1)
            .and_then(|index| grants.get(index))
            .ok_or_else(|| format!("No remembered grant {index}. Use /permissions remembered."))?;
        permissions.revoke(grant)?;
        let notice = format!(
            "Revoked grant {index} · {} of {}",
            grant.kind.name(),
            grant.target
        );
        return writeln!(output, "{}", agent.redact(&notice)).map_err(|error| error.to_string());
    }
    Err(USAGE.into())
}

pub(super) fn decide(
    permissions: &Permissions,
    redactor: &crate::redact::Redactor,
    request: &Request,
    waiting: usize,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<(), String> {
    let access = &request.access;
    let step = if waiting > 1 {
        format!(" · 1 of {waiting}")
    } else {
        String::new()
    };
    let place = access
        .place
        .as_ref()
        .map_or(String::new(), |place| format!(" in {place}"));
    write!(
        output,
        "Permission · main agent{step}\n{} {}{place}\n{}\n  1 Approve · {}\n  2 Approve and remember · {}, this conversation\n  3 Deny · {}\n",
        request.tool.to_uppercase(),
        redactor.text(&access.target),
        wording::access(access),
        wording::once(access),
        wording::covers(access, &request.tool),
        wording::DENY,
    )
    .map_err(|error| error.to_string())?;
    let decision = loop {
        write!(output, "Choose 1-3: ").map_err(|error| error.to_string())?;
        output.flush().map_err(|error| error.to_string())?;
        let Some(line) = read_line(input)? else {
            writeln!(output, "Input closed: denied.").map_err(|error| error.to_string())?;
            break Decision::Deny;
        };
        match line.trim() {
            "1" => break Decision::Once,
            "2" => break Decision::Remember,
            "3" => break Decision::Deny,
            _ => writeln!(output, "Type 1, 2 or 3.").map_err(|error| error.to_string())?,
        }
    };
    permissions.decide(request.id, decision)
}
