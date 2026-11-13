//! Command-line parsing. Malformed input is rejected here, before any setup,
//! configuration read, model request or file change.

mod actions;
pub mod completions;
pub mod grammar;
pub mod help;
#[cfg(test)]
mod tests;

use crate::backend::Routing;
use crate::effort::Effort;
use crate::permissions::Level;
use actions::{action, help_request};
use grammar::{Command, Opt};
use std::ffi::OsString;
use std::path::PathBuf;

pub struct Invocation {
    /// `-C DIR`: the working directory every command uses.
    pub directory: Option<PathBuf>,
    pub action: Action,
}

pub enum Action {
    Help(Vec<&'static str>),
    Version,
    Run(Run),
    Setup,
    Completions(String),
    Update {
        check: bool,
        json: bool,
    },
    Uninstall {
        yes: bool,
        json: bool,
    },
    Doctor {
        json: bool,
    },
    /// A management command: its family and subcommand, operands and options.
    Manage(Manage),
}

/// Per-run selection. `None` keeps the saved default or the resumed
/// conversation's own choice; nothing here is saved as a default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub routing: Option<Routing>,
    pub effort: Option<Effort>,
    pub permissions: Option<Level>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Resume {
    Pick,
    Last,
    Id(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Start {
    New,
    Resume(Resume),
    Init,
}

pub struct Run {
    pub start: Start,
    pub prompt: Option<String>,
    pub attachments: Vec<String>,
    pub plain: bool,
    pub json: bool,
    pub selection: Selection,
}

pub struct Manage {
    pub family: &'static str,
    pub action: &'static str,
    pub operands: Vec<String>,
    pub values: Vec<(&'static str, Option<String>)>,
}

impl Manage {
    pub fn flag(&self, long: &str) -> bool {
        self.values.iter().any(|(name, _)| *name == long)
    }

    pub fn value(&self, long: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(name, _)| *name == long)
            .and_then(|(_, value)| value.as_deref())
    }

    pub fn json(&self) -> bool {
        self.flag("json")
    }
}

/// A usage error: exit status 2, with the command whose help applies.
#[derive(Debug)]
pub struct Usage {
    pub message: String,
    pub path: Vec<&'static str>,
}

impl Usage {
    fn new(message: impl Into<String>, path: &[&'static str]) -> Self {
        Self {
            message: message.into(),
            path: path.to_vec(),
        }
    }

    pub fn text(&self) -> String {
        let command = if self.path.is_empty() {
            "jecode --help".to_owned()
        } else {
            format!("jecode {} --help", self.path.join(" "))
        };
        format!("jecode: {}\nRun `{command}` for usage.", self.message)
    }
}

/// How a command failed: invalid usage exits 2, any other failure 1.
#[derive(Debug)]
pub enum Failure {
    Usage(String),
    Error(String),
}

impl From<String> for Failure {
    fn from(error: String) -> Self {
        Self::Error(error)
    }
}

impl From<&str> for Failure {
    fn from(error: &str) -> Self {
        Self::Error(error.into())
    }
}

pub fn usage(message: impl Into<String>) -> Failure {
    Failure::Usage(message.into())
}

pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Invocation, Usage> {
    let tokens: Vec<String> = arguments
        .into_iter()
        .map(|value| {
            value
                .into_string()
                .map_err(|_| Usage::new("arguments must be valid UTF-8", &[]))
        })
        .collect::<Result<_, _>>()?;
    if let Some(path) = help_request(&tokens)? {
        return Ok(Invocation {
            directory: None,
            action: Action::Help(path),
        });
    }
    let mut command: &'static Command = &grammar::ROOT;
    let mut path: Vec<&'static str> = Vec::new();
    let mut values: Vec<(&'static Opt, Option<String>)> = Vec::new();
    let mut positionals: Vec<String> = Vec::new();
    let mut literal = false;
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        index += 1;
        if !literal && token == "--" {
            literal = true;
            continue;
        }
        if !literal && token.starts_with('-') && token.len() > 1 {
            if command.prompt
                && !positionals.is_empty()
                && !resume_id_only(command, &positionals, &values)
            {
                return Err(Usage::new(
                    format!(
                        "option {token} follows the prompt. Put options first, quote the prompt, or put -- before a prompt containing '{token}'"
                    ),
                    &path,
                ));
            }
            let (option, inline) = lookup(token, &path)?;
            if command.name == "resume" && option.long == "last" && !positionals.is_empty() {
                let message = if crate::sessions::valid_id(&positionals[0]) {
                    "cannot combine SESSION_ID with --last"
                } else {
                    "--last follows the prompt; put options first"
                };
                return Err(Usage::new(message, &path));
            }
            let value = match (option.value, inline) {
                (None, None) => None,
                (None, Some(_)) => {
                    return Err(Usage::new(
                        format!("--{} does not take a value", option.long),
                        &path,
                    ));
                }
                (Some(_), Some(value)) => Some(value),
                (Some(name), None) => {
                    let value = tokens.get(index).ok_or_else(|| {
                        Usage::new(format!("--{} requires {name}", option.long), &path)
                    })?;
                    if value.starts_with('-') && value != "-" {
                        return Err(Usage::new(
                            format!(
                                "--{} requires {name}; write --{}=VALUE for a value starting with '-'",
                                option.long, option.long
                            ),
                            &path,
                        ));
                    }
                    index += 1;
                    Some(value.clone())
                }
            };
            if option.long == "version" {
                if tokens.len() != 1 {
                    return Err(Usage::new("--version takes no other arguments", &[]));
                }
                return Ok(Invocation {
                    directory: None,
                    action: Action::Version,
                });
            }
            values.push((option, value));
            continue;
        }
        if !literal
            && positionals.is_empty()
            && let Some(sub) = command.subcommand(token)
        {
            command = sub;
            path.push(sub.name);
            continue;
        }
        if !literal && positionals.is_empty() && !command.subcommands.is_empty() && !command.prompt
        {
            return Err(Usage::new(
                format!("unknown command '{token}' for 'jecode {}'", path.join(" ")),
                &path,
            ));
        }
        positionals.push(token.clone());
    }
    if !command.subcommands.is_empty() && !command.prompt {
        let default = command.default.ok_or_else(|| {
            Usage::new(
                format!("'jecode {}' needs a command", path.join(" ")),
                &path,
            )
        })?;
        command = command.subcommand(default).expect("default subcommand");
        path.push(command.name);
    }
    let directory = check_options(command, &path, &values)?;
    let values: Vec<(&'static str, Option<String>)> = values
        .into_iter()
        .filter(|(option, _)| option.long != "cd")
        .map(|(option, value)| (option.long, value))
        .collect();
    let action = action(command, &path, positionals, values)?;
    Ok(Invocation { directory, action })
}

/// `resume ID` keeps accepting options until the prompt itself begins; with
/// `--last` the first positional already starts the prompt.
fn resume_id_only(
    command: &Command,
    positionals: &[String],
    values: &[(&'static Opt, Option<String>)],
) -> bool {
    command.name == "resume"
        && positionals.len() == 1
        && !values.iter().any(|(option, _)| option.long == "last")
}

fn lookup(token: &str, path: &[&'static str]) -> Result<(&'static Opt, Option<String>), Usage> {
    if let Some(long) = token.strip_prefix("--") {
        let (name, value) = match long.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (long, None),
        };
        let option = grammar::option_named(name)
            .ok_or_else(|| Usage::new(format!("unknown option --{name}"), path))?;
        return Ok((option, value));
    }
    let mut characters = token[1..].chars();
    let short = characters.next().unwrap();
    if characters.next().is_some() {
        return Err(Usage::new(
            format!(
                "unknown option {token}; short options take their value as a separate argument"
            ),
            path,
        ));
    }
    let option = grammar::option_short(short)
        .ok_or_else(|| Usage::new(format!("unknown option {token}"), path))?;
    Ok((option, None))
}

fn check_options(
    command: &Command,
    path: &[&'static str],
    values: &[(&'static Opt, Option<String>)],
) -> Result<Option<PathBuf>, Usage> {
    let name = if path.is_empty() {
        "jecode".to_owned()
    } else {
        format!("jecode {}", path.join(" "))
    };
    let mut directory = None;
    for (position, (option, value)) in values.iter().enumerate() {
        if !command.allows(option.long) {
            return Err(Usage::new(
                format!("--{} does not apply to '{name}'", option.long),
                path,
            ));
        }
        if !option.repeat
            && values[..position]
                .iter()
                .any(|(earlier, _)| earlier.long == option.long)
        {
            return Err(Usage::new(
                format!("--{} may only be supplied once", option.long),
                path,
            ));
        }
        if let Some(value) = value {
            if value.trim().is_empty() {
                return Err(Usage::new(
                    format!("--{} requires a non-empty value", option.long),
                    path,
                ));
            }
            if !option.choices.is_empty() && !option.choices.contains(&value.as_str()) {
                return Err(Usage::new(
                    format!(
                        "invalid --{} value '{value}'; use {}",
                        option.long,
                        option.choices.join(", ")
                    ),
                    path,
                ));
            }
        }
        if option.long == "cd" {
            directory = value.clone().map(PathBuf::from);
        }
    }
    Ok(directory)
}
