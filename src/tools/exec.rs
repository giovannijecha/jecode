//! Direct process execution with structured arguments and no command shell.

use super::shell;
use crate::cancel::Cancellation;
use crate::json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(test)]
mod tests;

pub(super) fn execute(
    context: shell::Context<'_>,
    arguments: &Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    let _ = shell::protect_paths(arguments)?;
    let (command, input, label) = command(context.root, context.temporary, arguments)?;
    shell::execute_command(context, arguments, cancellation, command, input, &label)
}

/// Builds the same native command used for logged and interactive execution.
/// The caller owns launch, supervision, and output retention.
pub(super) fn command(
    root: &Path,
    temporary: Option<&Path>,
    arguments: &Value,
) -> Result<(Command, Option<Vec<u8>>, String), String> {
    let executable = super::required_string(arguments, "executable")?;
    if executable.trim().is_empty() {
        return Err("executable must not be empty".into());
    }
    if executable.contains('\0') {
        return Err("executable must not contain NUL characters".into());
    }
    let args = match arguments.get("args") {
        None => Vec::new(),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "args must contain only strings".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err("args must be a list of strings".into()),
    };
    if args.iter().any(|argument| argument.contains('\0')) {
        return Err("args must not contain NUL characters".into());
    }
    let cwd = working_directory(root, arguments)?;
    let program = Path::new(executable);
    if !program.is_absolute()
        && (program.has_root()
            || matches!(
                program.components().next(),
                Some(std::path::Component::Prefix(_))
            ))
    {
        return Err("executable must be an absolute or cwd-relative path or a PATH name".into());
    }
    let program = if program.is_absolute() {
        program.to_path_buf()
    } else if program
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty())
    {
        cwd.join(program)
    } else {
        PathBuf::from(program)
    };
    let mut command = Command::new(program);
    command.args(&args).current_dir(cwd);
    crate::auth::isolate_environment(&mut command);
    if let Some(value) = arguments.get("env") {
        let Value::Object(env) = value else {
            return Err("env must be an object of string values or null".into());
        };
        for (name, value) in env {
            if name.is_empty() || name.contains('=') || name.contains('\0') {
                return Err("env contains an invalid variable name".into());
            }
            match value {
                Value::String(value) => {
                    command.env(name, value);
                }
                Value::Null => {
                    command.env_remove(name);
                }
                _ => return Err(format!("env.{name} must be a string or null")),
            }
        }
    }
    if let Some(temporary) = temporary {
        let temporary = crate::scratch::environment_path(temporary);
        command
            .env("JECODE_TMP", &temporary)
            .env("TMPDIR", &temporary);
        command.env("TEMP", &temporary).env("TMP", &temporary);
    }
    let input = match arguments.get("stdin") {
        None => None,
        Some(Value::String(text)) => Some(text.as_bytes().to_vec()),
        Some(_) => return Err("stdin must be a string".into()),
    };
    let label = crate::events::target("exec", arguments);
    Ok((command, input, label))
}

/// The canonical working directory: `cwd` absolute or project-relative, the
/// project root by default.
pub(super) fn working_directory(root: &Path, arguments: &Value) -> Result<PathBuf, String> {
    let cwd = match arguments.get("cwd") {
        None => root.to_path_buf(),
        Some(Value::String(path)) if !path.is_empty() => {
            let path = Path::new(path);
            if path.is_absolute() {
                path.to_path_buf()
            } else if path.has_root()
                || matches!(
                    path.components().next(),
                    Some(std::path::Component::Prefix(_))
                )
            {
                return Err("cwd must be an absolute or project-relative directory path".into());
            } else {
                root.join(path)
            }
        }
        Some(_) => return Err("cwd must be a nonempty directory path".into()),
    };
    let cwd = std::fs::canonicalize(&cwd)
        .map_err(|error| format!("Invalid cwd {}: {error}", cwd.display()))?;
    if !cwd.is_dir() {
        return Err(format!("cwd must be a directory: {}", cwd.display()));
    }
    Ok(cwd)
}
