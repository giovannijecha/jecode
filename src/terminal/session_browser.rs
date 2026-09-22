//! Human-readable local session cards and a choice bound to the displayed list.
#[cfg(test)]
#[path = "session_browser_tests.rs"]
mod tests;
use super::{
    Key, platform,
    style::{Row, Tone},
    text,
};
use crate::session::persistence::{self, Listed};
use std::{
    io::{self, IsTerminal, Write},
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) fn show(select: bool) -> io::Result<Option<String>> {
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    if select && !interactive {
        return Err(io::Error::other(
            "session selection needs an interactive terminal",
        ));
    }
    // No lease, authentication or history write occurs while browsing.
    let sessions = persistence::list()?;
    let mut terminal = interactive.then(platform::Terminal::open).transpose()?;
    let columns = match &terminal {
        Some(terminal) => terminal.size()?.0,
        None => 80,
    };
    let color = interactive && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty());
    let mut output = io::stdout().lock();
    for row in rows(&sessions, columns, SystemTime::now(), !interactive) {
        writeln!(output, "{}\r", row.paint(color))?;
    }
    if sessions.is_empty() {
        return Ok(None);
    }
    if !select {
        for line in text::wrap(
            "Run jecode resume to choose a session.",
            columns.saturating_sub(1),
        ) {
            writeln!(output, "{}\r", Row::new(line, Tone::Muted).paint(color))?;
        }
        return Ok(None);
    }
    let terminal = terminal
        .as_mut()
        .expect("interactive terminal checked above");
    for line in text::wrap(
        "Type a number, then Enter. Esc cancels.",
        columns.saturating_sub(1),
    ) {
        writeln!(output, "{}\r", Row::new(line, Tone::Muted).paint(color))?;
    }
    let _prompt = Prompt;
    let mut choice = Choice::default();
    let mut previous = None;
    loop {
        let width = terminal.size()?.0.saturating_sub(1).max(1);
        let prompt = clipped(&format!("Resume > {}", choice.input), width);
        if previous.as_ref() != Some(&prompt) {
            write!(
                output,
                "\r\x1b[2K{}",
                Row::new(&prompt, Tone::Accent).paint(color)
            )?;
            output.flush()?;
            previous = Some(prompt);
        }
        for key in terminal.poll()? {
            match choice.key(key, &sessions) {
                Decision::Pending => {}
                Decision::Cancel => return Ok(None),
                Decision::Selected(index) => {
                    let prompt = clipped(&format!("Resume > {}", index + 1), width);
                    write!(
                        output,
                        "\r\x1b[2K{}",
                        Row::new(prompt, Tone::Accent).paint(color)
                    )?;
                    return Ok(Some(sessions[index].id.clone()));
                }
                Decision::Invalid => {
                    let message = clipped("Choose a listed, readable session number.", width);
                    writeln!(
                        output,
                        "\r\x1b[2K{}\r",
                        Row::new(message, Tone::Error).paint(color)
                    )?;
                    previous = None;
                }
            }
        }
    }
}

// A short, ordinary scrollback prompt; no transcript repaint on resize.
struct Prompt;
impl Drop for Prompt {
    fn drop(&mut self) {
        let mut output = io::stdout().lock();
        let _ = output.write_all(b"\r\n");
        let _ = output.flush();
    }
}

#[derive(Default)]
struct Choice {
    input: String,
}
#[derive(Debug, Eq, PartialEq)]
enum Decision {
    Pending,
    Cancel,
    Selected(usize),
    Invalid,
}
impl Choice {
    fn key(&mut self, key: Key, sessions: &[Listed]) -> Decision {
        match key {
            Key::Escape | Key::Interrupt | Key::Quit => return Decision::Cancel,
            Key::Enter if self.input.is_empty() => return Decision::Cancel,
            Key::Enter => {
                let selected = self
                    .input
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| n.checked_sub(1));
                self.input.clear();
                return match selected {
                    Some(index) if sessions.get(index).is_some_and(|s| s.model.is_some()) => {
                        Decision::Selected(index)
                    }
                    _ => Decision::Invalid,
                };
            }
            Key::Backspace | Key::Delete => {
                self.input.pop();
            }
            Key::Text(value) if value == "q" => return Decision::Cancel,
            Key::Text(value) => {
                if value.is_empty()
                    || !value.bytes().all(|b| b.is_ascii_digit())
                    || self.input.len() + value.len() > 3
                {
                    self.input.clear();
                    return Decision::Invalid;
                }
                self.input.push_str(&value);
            }
            _ => {}
        }
        Decision::Pending
    }
}

fn rows(sessions: &[Listed], columns: usize, now: SystemTime, ids: bool) -> Vec<Row> {
    let width = columns.saturating_sub(1).max(1);
    let mut rows = vec![Row::blank()];
    let mut push = |value: String, tone| rows.push(Row::new(clipped(&value, width), tone));
    push("Saved sessions".into(), Tone::Heading);
    push("Most recently active first".into(), Tone::Muted);
    if sessions.is_empty() {
        push("No saved sessions yet.".into(), Tone::Muted);
    }
    for (index, session) in sessions.iter().enumerate() {
        push(String::new(), Tone::Text);
        let title = single_line(&session.title);
        let title = if title.is_empty() {
            "Untitled conversation"
        } else {
            &title
        };
        push(format!(" {}. {title}", index + 1), Tone::Heading);
        if let Some(model) = session.model {
            let workspace = session
                .workspace
                .as_ref()
                .map(|path| path.to_string_lossy());
            let workspace = workspace.as_deref().unwrap_or("Conversation only");
            let workspace = workspace.strip_prefix(r"\\?\").unwrap_or(workspace);
            let folder = session.workspace.as_ref().and_then(|path| path.file_name());
            let label = folder.map_or_else(
                || workspace.to_owned(),
                |folder| format!("{} · {workspace}", folder.to_string_lossy()),
            );
            push(format!("    {label}"), Tone::Accent);
            let unit = if session.turns == 1 { "turn" } else { "turns" };
            push(
                format!(
                    "    {} · {} {unit} · {}",
                    age(session.modified, now),
                    session.turns,
                    model.id()
                ),
                Tone::Muted,
            );
        } else {
            push("    Cannot resume; saved file kept.".into(), Tone::Error);
        }
        if ids {
            push(format!("    ID: {}", session.id), Tone::Muted);
        }
    }
    rows.push(Row::blank());
    rows
}

fn single_line(value: &str) -> String {
    text::safe(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn clipped(value: &str, width: usize) -> String {
    let value = text::safe(value).replace('\n', " ");
    if text::width(&value) <= width {
        return value;
    }
    let available = width.saturating_sub(1);
    let end = text::boundaries(&value)
        .into_iter()
        .take_while(|end| text::width(&value[..*end]) <= available)
        .last()
        .unwrap_or(0);
    format!("{}…", value[..end].trim_end())
}

fn age(modified: SystemTime, now: SystemTime) -> String {
    if modified == UNIX_EPOCH {
        return "Activity time unknown".into();
    }
    let seconds = now.duration_since(modified).unwrap_or_default().as_secs();
    match seconds {
        0..60 => "Just now".into(),
        60..3600 => format!("{}m ago", seconds / 60),
        3600..86400 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86400),
    }
}
