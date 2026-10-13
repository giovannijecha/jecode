//! Plain-chat `/terminal`: local inspection and stop control for this conversation.
//! It uses the observer, so the model's next poll is unchanged.

use crate::agent::Agent;
use crate::session::commands::terminal_stop_id;
use crate::tools::processes::{Observer, Process, Window};
use std::io::Write;

pub(super) fn print(agent: &Agent, arguments: &str, output: &mut impl Write) -> Result<(), String> {
    let observer = agent.processes();
    let text = if let Some(id) = terminal_stop_id(arguments) {
        if id.is_empty() {
            return Err("Usage: /terminal stop PROCESS_ID".into());
        }
        let process = observer.request_stop(id)?;
        if process.active() {
            format!(
                "Stop requested for {}; awaiting final status.\n",
                process.id
            )
        } else {
            format!("{} · {}\n", process.id, process.label())
        }
    } else if arguments.is_empty() {
        list(&observer)
    } else {
        details(&observer, arguments)?
    };
    write!(output, "{text}").map_err(|error| error.to_string())
}

fn list(observer: &Observer) -> String {
    let processes: Vec<_> = observer
        .processes()
        .unwrap_or_default()
        .into_iter()
        .filter(Process::active)
        .collect();
    if processes.is_empty() {
        return "No active background processes in this conversation.\n".into();
    }
    let mut text = format!("Active background processes ({}):\n", processes.len());
    for process in &processes {
        text.push_str(&format!(
            "  {}  running  {}\n",
            process.id,
            single_line(&process.command)
        ));
    }
    text.push_str("Use /terminal PROCESS_ID to show saved output, or /terminal stop PROCESS_ID to stop one.\n");
    text
}

fn details(observer: &Observer, id: &str) -> Result<String, String> {
    let process = observer
        .processes()
        .unwrap_or_default()
        .into_iter()
        .find(|process| process.id == id)
        .ok_or_else(|| {
            format!("No process {id} in this conversation. Use /terminal to list processes.")
        })?;
    let mut text = format!(
        "{} · {}\nCommand: {}\n",
        process.id,
        process.label(),
        single_line(&process.command)
    );
    if let Some(error) = &process.error {
        text.push_str(&format!("Error: {}\n", visible(error)));
    }
    for (stream, title) in ["Output", "Errors"].into_iter().enumerate() {
        text.push_str(&stream_section(observer, &process, stream, title));
    }
    Ok(text)
}

fn stream_section(observer: &Observer, process: &Process, stream: usize, title: &str) -> String {
    match observer.read(&process.id, stream, Window::Tail) {
        Ok(page) if page.length == 0 => format!("{title}: none\n"),
        Ok(page) => {
            let range = if page.start > 0 {
                format!(" (last {} of {} bytes)", page.end - page.start, page.length)
            } else {
                String::new()
            };
            let mut body = visible(&page.text);
            if !body.ends_with('\n') {
                body.push('\n');
            }
            format!("{title}{range}:\n{body}")
        }
        Err(error) => format!("{title}: {error}\n"),
    }
}

fn single_line(text: &str) -> String {
    visible(text).replace('\n', " ")
}

/// Keeps saved output from steering the console through control sequences.
fn visible(text: &str) -> String {
    text.replace("\r\n", "\n")
        .chars()
        .map(|character| match character {
            '\n' | '\t' => character,
            character if character.is_control() => '\u{fffd}',
            character => character,
        })
        .collect()
}

#[cfg(test)]
mod tests;
