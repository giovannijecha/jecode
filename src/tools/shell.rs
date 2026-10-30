use super::required_string;
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::process;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

#[cfg(test)]
mod budget_tests;
#[cfg(all(test, windows))]
mod windows_tests;

pub(super) struct Context<'a> {
    pub root: &'a Path,
    pub outputs: &'a crate::output::Store,
    pub temporary: Option<&'a Path>,
    pub processes: &'a super::processes::Registry,
    pub sandbox: Option<&'a crate::sandbox::Policy>,
}

pub(super) fn execute(
    context: Context<'_>,
    arguments: &Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    let text = required_string(arguments, "command")?;
    let _ = protect_paths(arguments)?;
    if text.trim().is_empty() {
        return Err("command must not be empty".into());
    }
    if text.contains('\0') {
        return Err("command must not contain NUL characters".into());
    }
    let command = command(context.root, text, context.temporary)?;
    execute_command(context, arguments, cancellation, command, None, text)
}

/// Shared output and process lifecycle for Bash and direct executable calls.
pub(super) fn execute_command(
    context: Context<'_>,
    arguments: &Value,
    cancellation: &Cancellation,
    mut command: Command,
    input: Option<Vec<u8>>,
    label: &str,
) -> Result<Value, String> {
    let Context {
        outputs,
        processes,
        sandbox,
        ..
    } = context;
    crate::sandbox::apply(&mut command, sandbox)?;
    let wait = super::processes::wait_argument(arguments)?;
    let output_limit = super::processes::output_limit(arguments)?;
    let timeout = timeout_argument(arguments)?;
    let logs = outputs.create()?;
    let stdout_ref = logs.stdout_ref;
    let stderr_ref = logs.stderr_ref;
    if let Some(wait) = wait {
        let streams = streams(outputs, [&stdout_ref, &stderr_ref])?;
        // The turn cancellation applies only to launch; the registry owns the process.
        return match process::start_logged_input(
            &mut command,
            input,
            cancellation,
            [logs.stdout, logs.stderr],
        ) {
            Ok(started) => processes.adopt(
                label,
                streams,
                started,
                super::processes::Adoption {
                    timeout,
                    wait,
                    output_limit,
                    cancellation,
                },
            ),
            Err(error) if !error.started => Err(error.message),
            Err(error) => Ok(unknown_outcome(
                &error.message,
                streams.map(|s| s.reference),
            )),
        };
    }
    let run = process::run_logged_input(
        &mut command,
        input,
        timeout,
        0,
        cancellation,
        [logs.stdout, logs.stderr],
    );
    let output = match run {
        Ok(output) => output,
        Err(error) if !error.started => return Err(error.message),
        Err(error) => return Ok(unknown_outcome(&error.message, [stdout_ref, stderr_ref])),
    };
    let previews = (|| {
        Ok::<_, String>((
            preview(&outputs.resolve(&stdout_ref)?, output_limit)?,
            preview(&outputs.resolve(&stderr_ref)?, output_limit)?,
        ))
    })();
    let ((stdout, stdout_truncated), (stderr, stderr_truncated)) = match previews {
        Ok(previews) => previews,
        Err(error) => return Ok(unknown_outcome(&error, [stdout_ref, stderr_ref])),
    };
    let truncated = stdout_truncated || stderr_truncated;
    let mut result = Value::object([
        ("stdout", Value::string(stdout)),
        ("stderr", Value::string(stderr)),
        (
            "exit_code",
            output.exit_code.map_or(Value::Null, Value::number),
        ),
        ("timed_out", Value::Bool(output.timed_out)),
        ("truncated", Value::Bool(truncated)),
        ("cancelled", Value::Bool(output.cancelled)),
        ("stdout_bytes", Value::number(output.stdout_bytes)),
        ("stderr_bytes", Value::number(output.stderr_bytes)),
        ("stdout_truncated", Value::Bool(stdout_truncated)),
        ("stderr_truncated", Value::Bool(stderr_truncated)),
        ("output_limit_bytes", Value::number(output_limit)),
        ("stdout_file", Value::string(stdout_ref)),
        ("stderr_file", Value::string(stderr_ref)),
    ]);
    if let Some(error) = output.input_error
        && let Value::Object(fields) = &mut result
    {
        fields.insert("outcome".into(), Value::string("unknown"));
        fields.insert(
            "error".into(),
            Value::string(format!(
                "The command started but input delivery failed: {error}. Execution may already have changed the workspace; inspect it before repeating the command."
            )),
        );
    }
    Ok(result)
}

pub(super) fn timeout_argument(arguments: &Value) -> Result<Option<Duration>, String> {
    arguments
        .get("timeout_seconds")
        .map(|value| {
            value
                .as_usize()
                .filter(|value| *value > 0)
                .map(|value| Duration::from_secs(value as u64))
                .ok_or_else(|| "timeout_seconds must be a positive integer".to_string())
        })
        .transpose()
}

pub(super) fn streams(
    outputs: &crate::output::Store,
    references: [&str; 2],
) -> Result<[super::processes::Stream; 2], String> {
    let make = |reference: &str| -> Result<super::processes::Stream, String> {
        Ok(super::processes::Stream {
            path: outputs.resolve(reference)?,
            reference: reference.to_owned(),
        })
    };
    Ok([make(references[0])?, make(references[1])?])
}

/// Reads only the model preview from the completed, redacted output file.
fn preview(path: &Path, limit: usize) -> Result<(String, bool), String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    if length <= limit as u64 {
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        return Ok((String::from_utf8_lossy(&bytes).into_owned(), false));
    }
    let head_length = limit.div_ceil(2);
    let tail_length = limit / 2;
    let mut head = vec![0; head_length];
    let mut tail = vec![0; tail_length];
    file.read_exact(&mut head)
        .map_err(|error| error.to_string())?;
    file.seek(SeekFrom::Start(length - tail_length as u64))
        .map_err(|error| error.to_string())?;
    file.read_exact(&mut tail)
        .map_err(|error| error.to_string())?;
    let head_end = match std::str::from_utf8(&head) {
        Ok(_) => head.len(),
        Err(error) if error.error_len().is_none() => error.valid_up_to(),
        Err(_) => head.len(),
    };
    let tail_start = tail.iter().take_while(|byte| **byte & 0xC0 == 0x80).count();
    let omitted = length - head_end as u64 - (tail.len() - tail_start) as u64;
    Ok((
        format!(
            "{}\n[... {omitted} saved bytes omitted; read the output file for the complete stream ...]\n{}",
            String::from_utf8_lossy(&head[..head_end]),
            String::from_utf8_lossy(&tail[tail_start..])
        ),
        true,
    ))
}

/// Declared workspace names whose preimages Bash may need. Command text is
/// never parsed to guess write effects.
pub(super) fn protect_paths(arguments: &Value) -> Result<Vec<String>, String> {
    let Some(value) = arguments.get("protect") else {
        return Ok(Vec::new());
    };
    value
        .as_array()
        .ok_or("protect must be a list of workspace paths")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "protect must contain only workspace paths".to_string())
        })
        .collect()
}

fn unknown_outcome(message: &str, [stdout_ref, stderr_ref]: [String; 2]) -> Value {
    Value::object([
        (
            "error",
            Value::string(format!(
                "The command started but its result or output could not be retained: {message}. Execution may already have changed the workspace; inspect it before repeating the command."
            )),
        ),
        ("outcome", Value::string("unknown")),
        ("stdout_file", Value::string(stdout_ref)),
        ("stderr_file", Value::string(stderr_ref)),
    ])
}

fn command(root: &Path, text: &str, temporary: Option<&Path>) -> Result<Command, String> {
    let mut command = crate::shell::command(text)?;
    command
        .current_dir(root)
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env_remove("JECODE_TMP")
        .env("TERM", "dumb");
    crate::auth::isolate_environment(&mut command);
    if let Some(temporary) = temporary {
        let temporary = crate::scratch::environment_path(temporary);
        command
            .env("JECODE_TMP", &temporary)
            .env("TMPDIR", &temporary)
            .env("TEMP", &temporary)
            .env("TMP", &temporary);
    }
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;
    use std::time::Instant;

    fn execute(
        root: &Path,
        arguments: &Value,
        cancellation: &Cancellation,
    ) -> Result<Value, String> {
        let outputs =
            crate::output::Store::new(root.join("logs"), crate::redact::Redactor::empty());
        let processes = crate::tools::processes::Registry::default();
        super::execute(
            Context {
                root,
                outputs: &outputs,
                temporary: None,
                processes: &processes,
                sandbox: None,
            },
            arguments,
            cancellation,
        )
    }

    fn run(root: &Path, text: &str, timeout: usize) -> Value {
        execute(
            root,
            &Value::object([
                ("command", Value::string(text)),
                ("timeout_seconds", Value::number(timeout)),
            ]),
            &Cancellation::default(),
        )
        .unwrap()
    }

    #[test]
    fn captures_both_streams_exit_code_and_working_directory() {
        let directory = Directory::new();
        std::fs::write(directory.path().join("fixture.txt"), "fixture").unwrap();
        let result = run(
            directory.path(),
            "cat fixture.txt; printf 'problem' >&2; exit 7",
            10,
        );
        assert_eq!(result.get("stdout").unwrap().as_str(), Some("fixture"));
        assert_eq!(result.get("stderr").unwrap().as_str(), Some("problem"));
        assert_eq!(result.get("exit_code").unwrap().as_usize(), Some(7));
        assert_eq!(result.get("timed_out"), Some(&Value::Bool(false)));
    }

    #[test]
    fn drains_large_output_without_unbounded_capture() {
        let directory = Directory::new();
        let result = run(
            directory.path(),
            "printf '%70000s' x; printf '%70000s' y >&2",
            10,
        );
        assert_eq!(
            result.get("output_limit_bytes"),
            Some(&Value::number(
                crate::tools::processes::DEFAULT_OUTPUT_LIMIT
            ))
        );
        assert!(
            result
                .get("stdout")
                .unwrap()
                .as_str()
                .unwrap()
                .contains("saved bytes omitted")
        );
        assert!(
            result
                .get("stderr")
                .unwrap()
                .as_str()
                .unwrap()
                .contains("saved bytes omitted")
        );
        assert_eq!(result.get("truncated"), Some(&Value::Bool(true)));
        assert_eq!(result.get("stdout_bytes"), Some(&Value::number(70000)));
        assert_eq!(result.get("stderr_bytes"), Some(&Value::number(70000)));
        assert_eq!(result.get("stdout_truncated"), Some(&Value::Bool(true)));
        assert_eq!(result.get("exit_code").unwrap().as_usize(), Some(0));
    }

    #[test]
    fn foreground_command_times_out_and_returns_partial_output() {
        let directory = Directory::new();
        let started = Instant::now();
        let result = run(directory.path(), "printf 'started'; sleep 10", 1);
        assert!(started.elapsed() < Duration::from_secs(6));
        assert_eq!(
            result.get("stdout").and_then(Value::as_str),
            Some("started"),
            "{result:?}"
        );
        assert_eq!(result.get("timed_out"), Some(&Value::Bool(true)));
    }

    #[test]
    fn shell_does_not_inherit_the_api_key_or_startup_script() {
        let directory = Directory::new();
        let command = command(directory.path(), "true", None).unwrap();
        for variable in ["OPENROUTER_API_KEY", "BASH_ENV", "ENV"] {
            assert!(
                command
                    .get_envs()
                    .any(|(key, value)| key == variable && value.is_none())
            );
        }
        let result = run(
            directory.path(),
            "printf '%s' \"${OPENROUTER_API_KEY-unset}\"",
            10,
        );
        assert_eq!(result.get("stdout").unwrap().as_str(), Some("unset"));
    }

    #[test]
    fn cancellation_stops_a_foreground_command_and_keeps_partial_output() {
        let directory = Directory::new();
        let cancellation = Cancellation::default();
        let signal = cancellation.clone();
        let marker = directory.path().join("started-marker");
        let logs = directory.path().join("logs");
        let worker = std::thread::spawn(move || {
            let started = Instant::now();
            let output_ready = || {
                marker.exists()
                    && std::fs::read_dir(&logs).is_ok_and(|entries| {
                        entries.flatten().any(|entry| {
                            entry.path().extension().is_some_and(|ext| ext == "stdout")
                                && std::fs::read(entry.path())
                                    .is_ok_and(|bytes| bytes == b"started")
                        })
                    })
            };
            while !output_ready() && started.elapsed() < Duration::from_secs(8) {
                std::thread::sleep(Duration::from_millis(10));
            }
            let observed = output_ready();
            signal.cancel();
            observed
        });
        let started = Instant::now();
        let result = execute(
            directory.path(),
            &Value::object([(
                "command",
                Value::string("printf 'started'; : > started-marker; sleep 20"),
            )]),
            &cancellation,
        )
        .unwrap();
        assert!(
            worker.join().unwrap(),
            "command output was not captured within 8 seconds"
        );
        assert!(started.elapsed() < Duration::from_secs(6));
        assert_eq!(
            result.get("stdout").and_then(Value::as_str),
            Some("started"),
            "{result:?}"
        );
        assert_eq!(result.get("cancelled"), Some(&Value::Bool(true)));
        assert_eq!(result.get("timed_out"), Some(&Value::Bool(false)));
    }
}
