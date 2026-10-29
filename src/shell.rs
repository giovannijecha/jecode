//! Jecode's small, deterministic command language. It never delegates parsing
//! to a host shell. External programs still use the host's process APIs.
#[path = "shell/builtins.rs"]
mod builtins;
#[path = "shell/flow.rs"]
mod flow;
#[path = "shell/parser.rs"]
mod parser;
#[path = "shell/process.rs"]
mod process;
use parser::parse;
#[path = "shell/lexer.rs"]
mod lexer;

use lexer::{Part, Token, Word};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const ENTRY: &str = "--internal-shell";

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
    variables: HashMap<String, String>,
    positional: Vec<String>,
    arg_zero: String,
    status: i32,
    exit: Option<i32>,
    stop_on_error: bool,
    jobs: Vec<std::process::Child>,
    last_background: Option<u32>,
}

#[derive(Default)]
struct Simple {
    words: Vec<Word>,
    redirects: Vec<Redirect>,
}

struct Redirect {
    stream: u8,
    mode: &'static str,
    target: Word,
}

impl Shell {
    fn new() -> Result<Self, String> {
        Ok(Self {
            cwd: std::env::current_dir().map_err(|error| error.to_string())?,
            variables: std::env::vars_os()
                .filter_map(|(name, value)| {
                    Some((name.into_string().ok()?, value.into_string().ok()?))
                })
                .collect(),
            positional: Vec::new(),
            arg_zero: "jecode-shell".into(),
            status: 0,
            exit: None,
            stop_on_error: false,
            jobs: Vec::new(),
            last_background: None,
        })
    }

    fn expand(&self, word: &Word) -> Result<String, String> {
        let mut result = String::new();
        for part in &word.0 {
            match part {
                Part::Literal(text) => result.push_str(text),
                Part::Status => result.push_str(&self.status.to_string()),
                Part::Positional(0) => result.push_str(&self.arg_zero),
                Part::Positional(index) => {
                    result.push_str(self.positional.get(index - 1).map_or("", String::as_str))
                }
                Part::AllArguments => result.push_str(&self.positional.join(" ")),
                Part::Pid => result.push_str(&std::process::id().to_string()),
                Part::LastBackground => result.push_str(
                    &self
                        .last_background
                        .map_or(String::new(), |pid| pid.to_string()),
                ),
                Part::Command(script) => result.push_str(&self.substitute(script)?),
                Part::Variable(name) => {
                    if let Some((name, fallback)) = name.split_once(":-") {
                        result.push_str(
                            self.variables
                                .get(name)
                                .filter(|v| !v.is_empty())
                                .map_or(fallback, String::as_str),
                        );
                    } else if let Some((name, fallback)) = name.split_once('-') {
                        result.push_str(self.variables.get(name).map_or(fallback, String::as_str));
                    } else {
                        result.push_str(self.variables.get(name).map_or("", String::as_str));
                    }
                }
            }
        }
        Ok(result)
    }

    fn substitute(&self, script: &str) -> Result<String, String> {
        let mut command = crate::shell::command(script)?;
        command.current_dir(&self.cwd).stdin(Stdio::null());
        for (name, value) in &self.variables {
            command.env(name, value);
        }
        let output = command
            .output()
            .map_err(|error| format!("command substitution: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "command substitution exited {}",
                output.status.code().unwrap_or(1)
            ));
        }
        if output.stdout.len() > 1024 * 1024 {
            return Err("command substitution exceeds 1 MiB".into());
        }
        String::from_utf8(output.stdout)
            .map(|text| text.trim_end_matches('\n').to_owned())
            .map_err(|_| "command substitution output must be UTF-8".into())
    }

    fn run(&mut self, script: &str) -> i32 {
        let tokens = match lexer::tokenize(script) {
            Ok(tokens) => tokens,
            Err(error) => {
                eprintln!("shell: {error}");
                return 2;
            }
        };
        self.run_tokens(&tokens)
    }

    fn run_tokens(&mut self, tokens: &[Token]) -> i32 {
        if matches!(tokens.first(), Some(Token::Op("("))) {
            return self.background_subshell(tokens);
        }
        if tokens.first().is_some_and(|token| literal(token, "for")) {
            return self.for_loop(tokens);
        }
        if tokens.first().is_some_and(|token| literal(token, "if")) {
            return self.if_command(tokens);
        }
        if tokens
            .first()
            .is_some_and(|token| literal(token, "while") || literal(token, "until"))
        {
            return self.loop_commands(tokens);
        }
        if let Some(index) = tokens.iter().enumerate().find_map(|(index, token)| {
            (index > 0
                && matches!(tokens[index - 1], Token::Op(";"))
                && (literal(token, "for")
                    || literal(token, "while")
                    || literal(token, "until")
                    || literal(token, "if")))
            .then_some(index)
        }) {
            let status = self.run_tokens(&tokens[..index]);
            if self.exit.is_some() || self.stop_on_error && status != 0 {
                return status;
            }
            return self.run_tokens(&tokens[index..]);
        }
        let groups = match parse(tokens) {
            Ok(groups) => groups,
            Err(error) => {
                eprintln!("shell: {error}");
                return 2;
            }
        };
        for (index, (separator, pipeline, background)) in groups.iter().enumerate() {
            let should_run = match *separator {
                "&&" => self.status == 0,
                "||" => self.status != 0,
                _ => true,
            };
            if !should_run {
                continue;
            }
            self.status = if *background {
                self.background(pipeline)
            } else {
                self.pipeline(pipeline)
            };
            if let Some(exit) = self.exit {
                return exit;
            }
            let conditional_continuation = groups
                .get(index + 1)
                .is_some_and(|(next, _, _)| matches!(*next, "&&" | "||"));
            if self.stop_on_error && self.status != 0 && !conditional_continuation {
                return self.status;
            }
        }
        self.status
    }

    fn pipeline(&mut self, stages: &[Simple]) -> i32 {
        if stages.len() > 1 {
            return match self.streaming_pipeline(stages) {
                Ok(status) => status,
                Err(error) => {
                    eprintln!("shell: {error}");
                    1
                }
            };
        }
        let mut input = None;
        for (index, stage) in stages.iter().enumerate() {
            let last = index + 1 == stages.len();
            let (status, output) = self.simple(stage, input.take(), !last);
            self.status = status;
            if !last {
                input = Some(output);
            }
        }
        self.status
    }

    fn simple(
        &mut self,
        simple: &Simple,
        pipeline_input: Option<Vec<u8>>,
        capture: bool,
    ) -> (i32, Vec<u8>) {
        let mut args: Vec<String> =
            match simple.words.iter().map(|word| self.expand(word)).collect() {
                Ok(args) => args,
                Err(error) => {
                    eprintln!("shell: {error}");
                    return (1, Vec::new());
                }
            };
        let mut local_env = Vec::new();
        while args.first().is_some_and(|arg| assignment(arg)) {
            let assignment = args.remove(0);
            let (name, value) = assignment.split_once('=').unwrap();
            local_env.push((name.to_owned(), value.to_owned()));
        }
        if args.is_empty() {
            for (name, value) in local_env {
                self.variables.insert(name, value);
            }
            return (0, Vec::new());
        }
        let mut input = pipeline_input;
        for redirect in &simple.redirects {
            if redirect.mode == "<" {
                let target = match self.expand(&redirect.target) {
                    Ok(target) => target,
                    Err(error) => {
                        eprintln!("shell: {error}");
                        return (1, Vec::new());
                    }
                };
                match fs::read(self.path(&target)) {
                    Ok(bytes) => input = Some(bytes),
                    Err(error) => {
                        eprintln!("shell: {error}");
                        return (1, Vec::new());
                    }
                }
            }
        }
        let (result, external) = if let Some(result) = builtins::run(
            self,
            &args,
            input.as_deref(),
            simple.redirects.is_empty() && !capture,
        ) {
            (result, false)
        } else {
            (
                self.external(&args, input.as_deref(), &local_env, &simple.redirects),
                true,
            )
        };
        let (status, stdout, stderr) = match result {
            Ok(result) => result,
            Err(error) if external => {
                eprintln!("shell: {error}");
                return (127, Vec::new());
            }
            Err(error) => (1, Vec::new(), format!("shell: {error}\n").into_bytes()),
        };
        if external {
            return (status, Vec::new());
        }
        let mut captured = Vec::new();
        for (stream, data) in [(1, stdout), (2, stderr)] {
            if stream == 1 && simple.redirects.iter().any(|r| r.mode == ">&2") {
                if let Err(error) = io::stderr()
                    .write_all(&data)
                    .and_then(|()| io::stderr().flush())
                {
                    eprintln!("shell: {error}");
                    return (1, Vec::new());
                }
                continue;
            }
            let redirect = simple
                .redirects
                .iter()
                .rev()
                .find(|r| r.stream == stream && r.mode != "<");
            if let Some(redirect) = redirect {
                let path = match self.expand(&redirect.target) {
                    Ok(target) => self.path(&target),
                    Err(error) => {
                        eprintln!("shell: {error}");
                        return (1, Vec::new());
                    }
                };
                let file = OpenOptions::new()
                    .write(true)
                    .create(true)
                    .append(redirect.mode == ">>")
                    .truncate(redirect.mode == ">")
                    .open(&path);
                match file.and_then(|mut file| file.write_all(&data)) {
                    Ok(()) => {}
                    Err(error) => {
                        eprintln!("shell: {}: {error}", path.display());
                        return (1, Vec::new());
                    }
                }
            } else if stream == 1 && capture {
                captured = data;
            } else {
                let output = if stream == 1 {
                    io::stdout()
                        .write_all(&data)
                        .and_then(|()| io::stdout().flush())
                } else {
                    io::stderr()
                        .write_all(&data)
                        .and_then(|()| io::stderr().flush())
                };
                if let Err(error) = output {
                    eprintln!("shell: {error}");
                    return (1, Vec::new());
                }
            }
        }
        (status, captured)
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

fn literal(token: &Token, expected: &str) -> bool {
    matches!(token, Token::Word(Word(parts)) if parts.as_slice() == [Part::Literal(expected.to_owned())])
}

#[cfg(test)]
#[path = "shell/tests.rs"]
mod tests;
