mod agent;
mod attachments;
mod bootstrap;
mod cancel;
#[cfg(test)]
mod checkpoint_bench;
mod checkpoints;
mod cli;
mod clipboard;
mod config;
mod console;
mod context;
mod copy;
mod disk;
mod effort;
mod events;
mod export;
mod file_change;
mod inbox;
mod input;
mod json;
mod openrouter;
mod output;
mod process;
mod redact;
mod scratch;
mod session;
mod sessions;
mod setup;
mod skills;
mod tools;
mod tui;
mod utf8;

#[cfg(test)]
mod test_support;

use cli::Action;
use std::process::ExitCode;

fn main() -> ExitCode {
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
            println!("jecode {}", env!("CARGO_PKG_VERSION"));
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
