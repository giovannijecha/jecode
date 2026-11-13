//! Management commands: the command-line face of Settings, providers,
//! models, saved conversations, skills, MCP and LSP. Each uses the same
//! stores, validation and redaction as the TUI and plain chat. Inspection
//! never opens a conversation, asks a model, starts a language server or
//! signs in.

pub mod doctor;
mod lsp;
pub(crate) mod mcp;
mod models;
mod providers;
mod sessions;
mod settings;
mod skills;
#[cfg(test)]
mod tests;

use crate::cli::{Failure, Manage, usage};
use crate::config::Store;
use crate::json::Value;
use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;

pub fn run(command: Manage) -> Result<(), Failure> {
    let context = Context::discover()?;
    let mut output = io::stdout().lock();
    let report = match command.family {
        "settings" => settings::run(&context, &command)?,
        "providers" => providers::run(&context, &command)?,
        "models" => models::run(&context, &command)?,
        "sessions" => sessions::run(&context, &command, &mut output)?,
        "skills" => skills::run(&context, &command)?,
        "mcp" => mcp::run(&context, &command)?,
        "lsp" => lsp::run(&context, &command)?,
        family => unreachable!("unknown management family {family}"),
    };
    report.print(command.json(), &mut output)
}

pub struct Context {
    pub store: Store,
    pub home: PathBuf,
}

impl Context {
    pub fn discover() -> Result<Self, String> {
        let store = Store::discover()?;
        let home = store
            .path()
            .parent()
            .ok_or("Could not locate the Jecode home")?
            .to_path_buf();
        Ok(Self { store, home })
    }

    /// Saved credentials for management output that includes user-authored text.
    pub fn redactor(&self) -> Result<crate::redact::Redactor, Failure> {
        let redactor = crate::bootstrap::account_redactor(&self.home, &mut io::stderr())?;
        if let Some(settings) = self.store.load()? {
            for key in settings.credentials.values() {
                redactor.add_secret(key.clone());
            }
            redactor.add_secret(settings.api_key.clone());
        }
        Ok(redactor)
    }

    #[cfg(test)]
    pub fn at(home: &std::path::Path) -> Self {
        Self {
            store: Store::new(home.to_path_buf()),
            home: home.to_path_buf(),
        }
    }
}

/// A command's result: the same data as text and as JSON. Commands that
/// already wrote their output return `Report::done()`.
pub struct Report {
    pub value: Value,
    pub text: String,
}

impl Report {
    pub fn new(value: Value, text: impl Into<String>) -> Self {
        Self {
            value,
            text: text.into(),
        }
    }

    pub fn done() -> Self {
        Self::new(Value::Null, "")
    }

    pub fn redact(self, redactor: &crate::redact::Redactor) -> Self {
        Self::new(redactor.value(&self.value), redactor.text(&self.text))
    }

    pub fn print(&self, json: bool, output: &mut impl Write) -> Result<(), Failure> {
        let text = if json {
            if self.value == Value::Null {
                return Ok(());
            }
            format!("{}\n", self.value.pretty())
        } else if self.text.is_empty() {
            return Ok(());
        } else if self.text.ends_with('\n') {
            self.text.clone()
        } else {
            format!("{}\n", self.text)
        };
        output
            .write_all(text.as_bytes())
            .and_then(|()| output.flush())
            .map_err(|error| Failure::Error(format!("Could not write output: {error}")))
    }
}

/// Asks before a destructive change. Without a terminal it never waits:
/// `--yes` is required instead.
pub fn confirm(yes: bool, question: &str, word: &str) -> Result<bool, Failure> {
    if yes {
        return Ok(true);
    }
    if !io::stdin().is_terminal() {
        let subject = question.lines().next().unwrap_or(question);
        return Err(usage(format!(
            "confirmation needed ({subject}); without a terminal, add --yes"
        )));
    }
    let mut status = io::stderr().lock();
    write!(
        status,
        "{question}\nType {word} to confirm [Enter = cancel]: "
    )
    .and_then(|()| status.flush())
    .map_err(|error| error.to_string())?;
    let line = crate::input::read_line(&mut io::stdin().lock())?.unwrap_or_default();
    Ok(line.trim() == word)
}

/// A secret from a hidden terminal prompt or piped stdin, never an argument.
pub fn secret(prompt: &str) -> Result<String, Failure> {
    let value = if io::stdin().is_terminal() {
        let mut input = io::stdin().lock();
        let mut status = io::stderr().lock();
        crate::setup::Prompter::secret(
            &mut crate::setup::Terminal::new(&mut input, &mut status),
            prompt,
        )?
        .ok_or("Cancelled; nothing was changed")?
    } else {
        let mut text = String::new();
        io::stdin()
            .lock()
            .take(64 * 1024 + 1)
            .read_to_string(&mut text)
            .map_err(|error| format!("Could not read the secret from stdin: {error}"))?;
        if text.len() > 64 * 1024 {
            return Err("The piped secret exceeds 64 KiB".into());
        }
        text
    };
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(usage("the secret is empty; nothing was changed"));
    }
    Ok(value)
}
