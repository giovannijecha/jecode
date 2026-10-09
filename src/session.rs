mod attach;
pub mod commands;
mod copy;
pub(crate) mod delete;
mod persistence;
mod settings;
pub(crate) mod temporary;
pub use settings::SessionConfig;

use crate::agent::Agent;
use crate::attachments::{Attachment, Prompt};
use crate::console::Console;
use crate::input::read_line;
use std::io::{BufRead, Write};

pub use commands::HELP;

/// A line-based chat whose first message carries `staged` attachments.
pub fn chat(
    agent: &mut Agent,
    config: &mut SessionConfig,
    input: &mut impl BufRead,
    output: &mut impl Write,
    status: &mut impl Write,
    staged: Vec<Attachment>,
) -> Result<(), String> {
    chat_inner(agent, config, input, output, status, None, staged)
}

pub fn chat_with_resume(
    agent: &mut Agent,
    config: &mut SessionConfig,
    input: &mut impl BufRead,
    output: &mut impl Write,
    status: &mut impl Write,
    id: Option<String>,
) -> Result<(), String> {
    chat_inner(agent, config, input, output, status, Some(id), Vec::new())
}

fn chat_inner(
    agent: &mut Agent,
    config: &mut SessionConfig,
    input: &mut impl BufRead,
    output: &mut impl Write,
    status: &mut impl Write,
    resume: Option<Option<String>>,
    mut staged: Vec<Attachment>,
) -> Result<(), String> {
    let previous = agent.sessions();
    agent.enable_sessions(
        config
            .store
            .path()
            .parent()
            .expect("configuration directory"),
    )?;
    writeln!(output, "Jecode - {}\nWorking directory: {}\nTools execute directly. Type a task, /help for commands, or /exit to quit.", agent.model(), std::env::current_dir().map_err(|error| error.to_string())?.display())
        .map_err(|error| error.to_string())?;
    if let Some(id) = resume {
        persistence::resume(agent, config, id.as_deref(), input, output, status)?;
    } else {
        persistence::hint(agent, output)?;
    }
    attach::transfer(agent, previous.as_ref(), &mut staged)?;
    if !staged.is_empty() {
        attach::announce(&staged, output)?;
    }
    loop {
        write!(output, "\n> ")
            .and_then(|_| output.flush())
            .map_err(|error| error.to_string())?;
        let Some(line) = read_line(input)? else {
            return agent.save_session();
        };
        let line = line.trim();
        let result = match line {
            "" if staged.is_empty() => continue,
            _ if attach::command(line).is_some() => {
                attach::stage(agent, attach::command(line).unwrap(), &mut staged, output)
            }
            "/exit" => return agent.save_session(),
            "/help" => {
                agent.record_local_details(
                    "/help",
                    "Commands and controls",
                    "notice",
                    &commands::COMMANDS
                        .iter()
                        .map(|command| (command.name.into(), command.description.into()))
                        .collect::<Vec<_>>(),
                );
                writeln!(output, "{}", commands::PLAIN_HELP).map_err(|error| error.to_string())
            }
            "/copy" => copy::print(agent, input, output),
            "/drafts" => writeln!(
                output,
                "Pending drafts are managed in the fullscreen terminal interface."
            )
            .map_err(|error| error.to_string()),
            "/resume" => {
                let previous = agent.sessions();
                persistence::resume(agent, config, None, input, output, status)
                    .and_then(|()| attach::transfer(agent, previous.as_ref(), &mut staged))
            }
            "/tmp" => temporary::print(agent, "", output),
            _ if line.starts_with("/tmp ") => temporary::print(agent, line[5..].trim(), output),
            _ if line.starts_with("/resume ") => {
                let previous = agent.sessions();
                persistence::resume(agent, config, Some(line[8..].trim()), input, output, status)
                    .and_then(|()| attach::transfer(agent, previous.as_ref(), &mut staged))
            }
            "/export" => agent.archive().save().and_then(|path| {
                agent.record_local(
                    "/export",
                    &format!("Exported conversation to {}", path.display()),
                );
                writeln!(output, "Exported conversation to {}", path.display())
                    .map_err(|error| error.to_string())
            }),
            "/clear" | "/new" => {
                agent.start_new(config.settings.model.clone(), config.settings.effort)?;
                agent.record_local("/new", "New conversation · saved defaults applied");
                writeln!(output, "Conversation cleared.").map_err(|error| error.to_string())
            }
            "/setup" | "/settings" => config.configure(agent, input, output).map(|()| {
                agent.record_local(line, "Settings closed · current conversation kept");
            }),
            "/effort" => config.change_effort(agent, None, input, output),
            _ if line.starts_with("/effort ") => {
                config.change_effort(agent, Some(line[8..].trim()), input, output)
            }
            "/model" => config.change_model(agent, None, input, output),
            _ if line.starts_with("/model ") => {
                config.change_model(agent, Some(line[7..].trim()), input, output)
            }
            _ if line.starts_with('/') => {
                agent.record_local_details(line, &commands::unknown(line), "error", &[]);
                writeln!(output, "Unknown command. Use /help to see commands.")
                    .map_err(|error| error.to_string())
            }
            _ => {
                let prompt = Prompt::new(line.to_owned(), staged.clone());
                let before = agent.archive().messages.lock().unwrap().len();
                let result = agent.run_turn(prompt.clone(), &mut Console::new(output, status));
                let accepted =
                    agent.archive().messages.lock().unwrap().get(before) == Some(&prompt.message());
                if accepted {
                    staged.clear();
                    result.and(attach::save(agent, &staged))
                } else {
                    result
                }
            }
        };
        let result = result.and(agent.save_session());
        if let Err(error) = result {
            if line.starts_with('/') {
                agent.record_local_details(line, &error, "error", &[]);
            }
            writeln!(status, "{}", agent.redact(&error)).map_err(|error| error.to_string())?;
        }
    }
}

#[cfg(test)]
mod persistence_tests;
#[cfg(test)]
mod temporary_tests;
#[cfg(test)]
mod tests;
