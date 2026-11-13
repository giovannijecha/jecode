mod agent;
mod ask;
mod attachments;
mod auth;
mod backend;
mod bootstrap;
mod browser;
mod cancel;
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
mod install;
mod json;
mod lsp;
mod manage;
mod mcp;
mod native;
mod output;
mod path_filter;
mod permissions;
mod process;
mod pty;
mod redact;
mod sandbox;
mod scratch;
mod selection;
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

use cli::{Action, Failure};
use std::process::ExitCode;

fn shell_executable() -> Result<std::path::PathBuf, String> {
    native::helper("shell")
}

fn main() -> ExitCode {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--internal-shell")) {
        return ExitCode::from(shell::run_entry(std::env::args_os().skip(2)) as u8);
    }
    identity::capture();
    let invocation = match cli::parse(std::env::args_os().skip(1)) {
        Ok(invocation) => invocation,
        Err(usage) => {
            eprintln!("{}", usage.text());
            return ExitCode::from(2);
        }
    };
    if let Some(directory) = &invocation.directory
        && let Err(error) = std::env::set_current_dir(directory)
    {
        eprintln!("jecode: cannot use -C {}: {error}", directory.display());
        return ExitCode::from(2);
    }
    let result: Result<(), Failure> = match invocation.action {
        Action::Help(path) => {
            print!("{}", cli::help::text(&path));
            Ok(())
        }
        Action::Version => {
            println!("{}", identity::version_line());
            Ok(())
        }
        Action::Completions(shell) => {
            print!("{}", cli::completions::script(&shell));
            Ok(())
        }
        Action::Setup => bootstrap::configure().map_err(Failure::from),
        Action::Run(run) => bootstrap::run(run),
        Action::Doctor { json } => manage::doctor::run(json),
        Action::Update { check, json } => install::update(check, json),
        Action::Uninstall { yes, json } => install::uninstall(yes, json),
        Action::Manage(command) => manage::run(command),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(message)) => {
            eprintln!("jecode: {message}");
            ExitCode::from(2)
        }
        Err(Failure::Error(error)) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
