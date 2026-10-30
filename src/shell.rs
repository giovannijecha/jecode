//! Jecode's small, deterministic command language. It never delegates parsing
//! to a host shell. External programs still use the host's process APIs.
#[path = "shell/builtins.rs"]
mod builtins;
#[path = "shell/flow.rs"]
mod flow;
#[path = "shell/lexer.rs"]
mod lexer;
#[path = "shell/parser.rs"]
mod parser;
#[path = "shell/process.rs"]
mod process;
#[path = "shell/streams.rs"]
mod streams;
#[path = "shell/variables.rs"]
mod variables;

use lexer::{Part, Word};
use parser::{Command as Node, Compound, List, Redirect, Simple, parse};
use std::cell::Cell;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use streams::{Io, Sink, Source};
use variables::Variables;

const ENTRY: &str = "--internal-shell";
const SUBSTITUTION_LIMIT: usize = 1024 * 1024;

pub fn command(script: &str) -> Result<Command, String> {
    let executable = executable()?;
    let mut command = Command::new(executable);
    command.args([ENTRY, "-c", script]);
    Ok(command)
}

pub fn executable() -> Result<PathBuf, String> {
    crate::shell_executable()
}

/// Called before ordinary CLI parsing. The private entry accepts `-c SCRIPT`
/// or a script path, and optionally `-s` for a script from stdin.
pub fn run_entry(args: impl Iterator<Item = OsString>) -> i32 {
    let mut args = args.peekable();
    while args
        .peek()
        .is_some_and(|arg| arg == "--noprofile" || arg == "--norc")
    {
        args.next();
    }
    let args: Vec<_> = args.collect();
    let (script, positional, arg_zero) = match args.as_slice() {
        [flag, script, positional @ ..] if flag == "-c" => {
            let (arg_zero, positional) = match positional {
                [first, rest @ ..] => (first.to_string_lossy().into_owned(), rest),
                [] => ("jecode-shell".into(), &[][..]),
            };
            (script.to_string_lossy().into_owned(), positional, arg_zero)
        }
        [flag] if flag == "-s" => {
            let mut script = String::new();
            if let Err(error) = io::stdin().read_to_string(&mut script) {
                eprintln!("shell: {error}");
                return 2;
            }
            (script, &[][..], "jecode-shell".into())
        }
        [path, positional @ ..] => match fs::read_to_string(path) {
            Ok(script) => (script, positional, path.to_string_lossy().into_owned()),
            Err(error) => {
                eprintln!("shell: {}: {error}", path.to_string_lossy());
                return 2;
            }
        },
        _ => {
            eprintln!("shell: expected -c SCRIPT or a script path");
            return 2;
        }
    };
    let mut shell = match Shell::new() {
        Ok(shell) => shell,
        Err(error) => {
            eprintln!("shell: {error}");
            return 2;
        }
    };
    shell.positional = positional
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    shell.arg_zero = arg_zero;
    shell.run(&script)
}

struct Shell {
    cwd: PathBuf,
    variables: Variables,
    positional: Vec<String>,
    arg_zero: String,
    status: i32,
    exit: Option<i32>,
    stop_on_error: bool,
    /// Nesting depth of `if`/`while`/`until` conditions, where `set -e` is off.
    conditions: usize,
    jobs: Vec<std::process::Child>,
    last_background: Option<u32>,
    /// Status of the last command substitution in the current command.
    substitution: Cell<Option<i32>>,
    /// A closed output pipe ends this execution scope even when `set -e` is off.
    broken_output: bool,
}

impl Shell {
    fn new() -> Result<Self, String> {
        let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
        Ok(Self::with(cwd, Variables::from_environment()))
    }

    fn with(cwd: PathBuf, variables: Variables) -> Self {
        Self {
            cwd,
            variables,
            positional: Vec::new(),
            arg_zero: "jecode-shell".into(),
            status: 0,
            exit: None,
            stop_on_error: false,
            conditions: 0,
            jobs: Vec::new(),
            last_background: None,
            substitution: Cell::new(None),
            broken_output: false,
        }
    }

    /// A subshell: a copy of this shell's state whose changes stay local.
    fn fork(&self) -> Self {
        Self {
            positional: self.positional.clone(),
            arg_zero: self.arg_zero.clone(),
            status: self.status,
            stop_on_error: self.stop_on_error,
            conditions: self.conditions,
            last_background: self.last_background,
            ..Self::with(self.cwd.clone(), self.variables.clone())
        }
    }

    fn run(&mut self, script: &str) -> i32 {
        self.run_script(script, &Io::standard())
    }

    fn run_script(&mut self, script: &str, io: &Io) -> i32 {
        let list = match lexer::tokenize(script).and_then(|tokens| parse(script, &tokens)) {
            Ok(list) => list,
            Err(error) => {
                io.report(&error);
                return 2;
            }
        };
        let status = self.run_list(&list, io);
        self.exit.unwrap_or(status)
    }

    fn expand(&self, word: &Word, io: &Io) -> Result<String, String> {
        Ok(self.fields(word, io)?.join(" "))
    }

    /// Expands words into arguments; `"$@"` yields one per positional argument.
    fn words(&self, words: &[Word], io: &Io) -> Result<Vec<String>, String> {
        let mut result = Vec::with_capacity(words.len());
        for word in words {
            result.extend(self.fields(word, io)?);
        }
        Ok(result)
    }

    fn fields(&self, word: &Word, io: &Io) -> Result<Vec<String>, String> {
        let mut fields = Vec::new();
        let mut current = String::new();
        for part in &word.0 {
            if *part == Part::AllArguments {
                let mut arguments = self.positional.iter();
                if let Some(first) = arguments.next() {
                    current.push_str(first);
                    for argument in arguments {
                        fields.push(std::mem::take(&mut current));
                        current.push_str(argument);
                    }
                }
            } else {
                self.expand_part(part, &mut current, io)?;
            }
        }
        if word.0 != [Part::AllArguments] || !self.positional.is_empty() {
            fields.push(current);
        }
        Ok(fields)
    }

    fn expand_part(&self, part: &Part, result: &mut String, io: &Io) -> Result<(), String> {
        match part {
            Part::Literal(text) => result.push_str(text),
            Part::Status => result.push_str(&self.status.to_string()),
            Part::Positional(0) => result.push_str(&self.arg_zero),
            Part::Positional(index) => {
                result.push_str(self.positional.get(index - 1).map_or("", String::as_str))
            }
            Part::AllArguments => result.push_str(&self.positional.join(" ")),
            Part::Pid => result.push_str(&std::process::id().to_string()),
            Part::LastBackground => {
                if let Some(pid) = self.last_background {
                    result.push_str(&pid.to_string());
                }
            }
            Part::Command(script) => result.push_str(&self.substitute(script, io)?),
            Part::Variable(name) => {
                let value = if let Some((name, fallback)) = name.split_once(":-") {
                    self.variables
                        .get(name)
                        .filter(|value| !value.is_empty())
                        .unwrap_or(fallback)
                } else if let Some((name, fallback)) = name.split_once('-') {
                    self.variables.get(name).unwrap_or(fallback)
                } else {
                    self.variables.get(name).unwrap_or("")
                };
                result.push_str(value);
            }
        }
        Ok(())
    }

    /// Runs `$(script)` in a child shell. Its output is substituted whatever
    /// its status, which becomes `$?` for an assignment-only command.
    fn substitute(&self, script: &str, io: &Io) -> Result<String, String> {
        let mut command = crate::shell::command(script)?;
        let stdin = io.stdin.stdio().map_err(|error| error.to_string())?;
        let stderr = io.stderr.stdio(2).map_err(|error| error.to_string())?;
        command
            .current_dir(&self.cwd)
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(stderr);
        for (name, value) in self.variables.iter() {
            command.env(name, value);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("command substitution: {error}"))?;
        let mut output = Vec::new();
        let read = child.stdout.take().map(|stdout| {
            stdout
                .take(SUBSTITUTION_LIMIT as u64 + 1)
                .read_to_end(&mut output)
        });
        if output.len() > SUBSTITUTION_LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            return Err("command substitution exceeds 1 MiB".into());
        }
        let status = child
            .wait()
            .map_err(|error| format!("command substitution: {error}"))?;
        read.transpose()
            .map_err(|error| format!("command substitution: {error}"))?;
        self.substitution.set(Some(status.code().unwrap_or(1)));
        String::from_utf8(output)
            .map(|text| text.trim_end_matches('\n').to_owned())
            .map_err(|_| "command substitution output must be UTF-8".into())
    }

    fn path(&self, argument: &str) -> PathBuf {
        let path = Path::new(argument);
        if path.is_absolute() {
            path.to_owned()
        } else {
            self.cwd.join(path)
        }
    }
}

fn assignment(value: &str) -> bool {
    let Some((name, _)) = value.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name.chars().enumerate().all(|(index, c)| {
            c == '_'
                || if index == 0 {
                    c.is_ascii_alphabetic()
                } else {
                    c.is_ascii_alphanumeric()
                }
        })
}

#[cfg(test)]
#[path = "shell/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "shell/tests_runtime.rs"]
mod tests_runtime;
#[cfg(test)]
#[path = "shell/tests_syntax.rs"]
mod tests_syntax;
