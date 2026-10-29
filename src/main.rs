mod agent;
mod ask;
mod attachments;
mod auth;
mod backend;
mod bootstrap;
mod browser;
mod cancel;
#[cfg(test)]
mod checkpoint_bench;
mod checkpoints;
mod cli;
mod clipboard;
mod collaboration;
mod config;
mod console;
mod context;
mod conversation;
mod copy;
mod disk;
mod effort;
mod events;
mod export;
mod file_change;
mod http;
mod identity;
mod inbox;
mod input;
mod json;
mod native;
mod output;
mod path_filter;
mod permissions;
mod process;
mod project_map;
mod pty;
mod redact;
mod sandbox;
mod scratch;
mod session;
mod sessions;
mod setup;
mod shell;
mod skills;
mod todo;
mod tools;
mod tui;
mod utf8;
mod web;

#[cfg(test)]
mod test_support;

use cli::Action;
use std::process::ExitCode;

fn shell_executable() -> Result<std::path::PathBuf, String> {
    native::helper("shell")
}

fn main() -> ExitCode {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--internal-shell")) {
        return ExitCode::from(shell::run_entry(std::env::args_os().skip(2)) as u8);
    }
    identity::capture();
    let action = match cli::parse(std::env::args_os().skip(1)) {
        Ok(action) => action,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let result = match action {
        Action::Help => {
            cli::print_help();
            Ok(())
        }
        Action::Version => {
            println!("{}", identity::version_line());
            Ok(())
        }
        Action::Setup => bootstrap::configure(),
        Action::Resume { id, plain } => bootstrap::resume(id, plain),
        Action::Run {
            model,
            prompt,
            attachments,
            plain,
        } => bootstrap::run(model, prompt, attachments, plain),
        Action::Checkpoints(words) => bootstrap::checkpoints(&words),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
