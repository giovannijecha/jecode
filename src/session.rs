mod agents;
mod ask;
mod attach;
mod checkpoints;
pub mod commands;
mod copy;
pub(crate) mod delete;
mod permission;
mod persistence;
mod processes;
mod settings;
pub(crate) mod temporary;
mod todo;
pub use settings::SessionConfig;

use crate::agent::Agent;
use crate::attachments::{Attachment, Prompt};
use crate::console::Console;
use crate::events::{Event, EventSink};
use crate::input::read_line;
use std::io::{BufRead, Write};

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
        if agent.has_pending_question() {
            resume_question(agent, input, output, status)?;
        }
        if agent.has_open_collaboration_report() {
            run_plain(agent, Work::Collaboration, input, output, status)?;
        }
    } else {
        persistence::hint(agent, output)?;
    }
    attach::transfer(agent, previous.as_ref(), &mut staged)?;
    if !staged.is_empty() {
        attach::announce(&staged, output)?;
    }
    loop {
        if agent.has_pending_question() {
            resume_question(agent, input, output, status)?;
        }
        if agent.has_open_collaboration_report() {
            run_plain(agent, Work::Collaboration, input, output, status)?;
        }
        write!(output, "\n> ")
            .and_then(|_| output.flush())
            .map_err(|error| error.to_string())?;
        let Some(raw_line) = read_line(input)? else {
            return agent.save_session();
        };
        let line = raw_line.trim();
        if line.is_empty() && staged.is_empty() {
            continue;
        }
        let (name, arguments) = commands::split(line).unwrap_or(("", ""));
        let name = commands::canonical(name).to_ascii_lowercase();
        let direct_write = name == "/terminal"
            && arguments
                .split_whitespace()
                .next()
                .is_some_and(|action| action.eq_ignore_ascii_case("write"));
        let terminal_arguments = if direct_write {
            raw_line
                .trim_end_matches(['\r', '\n'])
                .trim_start()
                .split_once(char::is_whitespace)
                .map_or(arguments, |(_, rest)| rest.trim_start())
        } else {
            arguments
        };
        let result = match (name.as_str(), arguments) {
            ("/exit", "") => return agent.save_session(),
            ("/help", "") => {
                agent.record_local_details(
                    "/help",
                    "Commands and controls",
                    "notice",
                    &commands::COMMANDS
                        .iter()
                        .filter(|command| command.plain_usage.is_some())
                        .map(|command| (command.name.into(), command.description(false).into()))
                        .collect::<Vec<_>>(),
                );
                writeln!(output, "{}", commands::help(false)).map_err(|error| error.to_string())
            }
            ("/skills", "") => agent.skills().and_then(|skills| {
                let (result, details) = commands::skills_catalog(
                    skills
                        .into_iter()
                        .map(|skill| (skill.name, skill.description)),
                    agent.skill_warnings(),
                );
                agent.record_local_details("/skills", &result, "notice", &details);
                writeln!(output, "{}", agent.redact(&result)).map_err(|error| error.to_string())?;
                for (name, description) in details {
                    writeln!(
                        output,
                        "{}  {}",
                        agent.redact(&name),
                        agent.redact(&description)
                    )
                    .map_err(|error| error.to_string())?;
                }
                Ok(())
            }),
            ("/copy", "") => copy::print(agent, input, output),
            ("/drafts", "") => writeln!(
                output,
                "Pending drafts are managed in the fullscreen terminal interface."
            )
            .map_err(|error| error.to_string()),
            ("/resume", "") => {
                let previous = agent.sessions();
                persistence::resume(agent, config, None, input, output, status)
                    .and_then(|()| attach::transfer(agent, previous.as_ref(), &mut staged))
            }
            ("/terminal", _) => processes::print(agent, terminal_arguments, output),
            ("/agents", _) => agents::print(agent, arguments, output),
            ("/todo", "") => todo::print(agent, output),
            ("/tmp", _) => temporary::print(agent, arguments, output),
            ("/checkpoints", _) => checkpoints::print(agent, arguments, output),
            ("/resume", _) => {
                let previous = agent.sessions();
                persistence::resume(agent, config, Some(arguments), input, output, status)
                    .and_then(|()| attach::transfer(agent, previous.as_ref(), &mut staged))
            }
            ("/export", "") => agent.archive().save().and_then(|path| {
                agent.record_local(
                    "/export",
                    &format!("Exported conversation to {}", path.display()),
                );
                writeln!(output, "Exported conversation to {}", path.display())
                    .map_err(|error| error.to_string())
            }),
            ("/new", "") => {
                agent.start_new_selection(
                    &config.settings.backend_id,
                    config.settings.model.clone(),
                    config.settings.effort,
                )?;
                agent.record_local("/new", "New conversation · saved defaults applied");
                writeln!(output, "Conversation cleared.").map_err(|error| error.to_string())
            }
            ("/settings", "") => config.configure(agent, input, output).map(|()| {
                agent.record_local(line, "Settings closed · current conversation kept");
            }),
            ("/routing", "") => config.change_routing(agent, input, output),
            ("/permissions", _) => permission::command(agent, arguments, output),
            ("/effort", "") => config.change_effort(agent, None, input, output),
            ("/effort", _) => config.change_effort(agent, Some(arguments), input, output),
            ("/model", "") => config.change_model(agent, None, input, output),
            ("/model", _) => config.change_model(agent, Some(arguments), input, output),
            ("/init", "") => {
                let prompt = Prompt::plain(commands::INIT_PROMPT);
                run_plain(agent, Work::Turn(prompt), input, output, status)
            }
            (name, _) if name.starts_with('/') => {
                agent.record_local_details(line, &commands::unknown(line), "error", &[]);
                writeln!(output, "Unknown command. Use /help to see commands.")
                    .map_err(|error| error.to_string())
            }
            _ => {
                let prompt = Prompt::new(line.to_owned(), staged.clone());
                let before = agent.archive().messages.lock().unwrap().len();
                let result = run_plain(agent, Work::Turn(prompt.clone()), input, output, status);
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
                if direct_write {
                    agent.record_local_details(
                        "/terminal write [direct input]",
                        "Direct terminal input failed",
                        "error",
                        &[],
                    );
                } else {
                    agent.record_local_details(line, &error, "error", &[]);
                }
            }
            writeln!(status, "{}", agent.redact(&error)).map_err(|error| error.to_string())?;
        }
    }
}

/// A failed answer or continuation is reported and the chat stays open.
fn resume_question(
    agent: &mut Agent,
    input: &mut impl BufRead,
    output: &mut impl Write,
    status: &mut impl Write,
) -> Result<(), String> {
    let result =
        run_plain(agent, Work::Question, input, output, status).and_then(|()| agent.save_session());
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            writeln!(status, "{}", agent.redact(&error)).map_err(|error| error.to_string())
        }
    }
}

enum Work {
    Turn(Prompt),
    Question,
    Collaboration,
}

fn run_plain(
    agent: &mut Agent,
    work: Work,
    input: &mut impl BufRead,
    output: &mut impl Write,
    status: &mut impl Write,
) -> Result<(), String> {
    let questions = agent.questions();
    let permissions = agent.permissions();
    let session = agent.sessions();
    let redactor = agent.redactor();
    questions.set_available(true);
    permissions.set_surface(crate::permissions::Surface::Inline);
    let mut console = Console::new(output, status);
    let mut sink = |event: Event| -> Result<(), String> {
        if let Event::UserQuestion { request } = &event {
            return ask::answer(
                &questions,
                request,
                session.as_ref().ok_or("Question session is unavailable")?,
                input,
                console.output(),
            );
        }
        if let Event::PermissionRequest { request, waiting } = &event {
            return permission::decide(
                &permissions,
                &redactor,
                request,
                *waiting,
                input,
                console.output(),
            );
        }
        console.emit(event)
    };
    let result = match work {
        Work::Turn(prompt) => agent.run_turn(prompt, &mut sink),
        Work::Question => agent.continue_question(&mut sink).map(|_| ()),
        Work::Collaboration => agent.continue_collaboration(&mut sink).map(|_| ()),
    };
    questions.set_available(false);
    permissions.set_surface(crate::permissions::Surface::None);
    result
}

#[cfg(test)]
mod ask_tests;
#[cfg(test)]
mod checkpoint_tests;
#[cfg(test)]
mod permission_tests;
#[cfg(test)]
mod persistence_tests;
#[cfg(test)]
mod temporary_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod work_tests;
