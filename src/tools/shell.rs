use super::{OUTPUT_LIMIT, required_string};
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::process;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[cfg(all(test, windows))]
mod windows_tests;

pub(super) struct Context<'a> {
    pub bash: &'a Path,
    pub root: &'a Path,
    pub outputs: &'a crate::output::Store,
    pub temporary: Option<&'a Path>,
    pub processes: &'a super::processes::Registry,
}

pub(super) fn execute(
    context: Context<'_>,
    arguments: &Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    let Context {
        bash,
        root,
        outputs,
        temporary,
        processes,
    } = context;
    let text = required_string(arguments, "command")?;
    let _ = protect_paths(arguments)?;
    if text.trim().is_empty() {
        return Err("command must not be empty".into());
    }
    let wait = super::processes::wait_argument(arguments)?;
    let timeout = arguments
        .get("timeout_seconds")
        .map(|value| {
            value
                .as_usize()
                .filter(|value| *value > 0)
                .map(|value| Duration::from_secs(value as u64))
                .ok_or("timeout_seconds must be a positive integer")
        })
        .transpose()?;
    let logs = outputs.create()?;
    let stdout_ref = logs.stdout_ref;
    let stderr_ref = logs.stderr_ref;
    let mut command = command(bash, root, text, temporary);
    if let Some(wait) = wait {
        let streams = [
            super::processes::Stream {
                path: outputs.resolve(&stdout_ref)?,
                reference: stdout_ref,
            },
            super::processes::Stream {
                path: outputs.resolve(&stderr_ref)?,
                reference: stderr_ref,
            },
        ];
        // The turn cancellation applies only to launch; the registry owns the process.
        return match process::start_logged(&mut command, cancellation, [logs.stdout, logs.stderr]) {
            Ok(started) => processes.adopt(
                text,
                streams,
                started,
                super::processes::Adoption {
                    timeout,
                    wait,
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
    let output = match process::run_logged(
        &mut command,
        timeout,
        OUTPUT_LIMIT,
        cancellation,
        [logs.stdout, logs.stderr],
    ) {
        Ok(output) => output,
        Err(error) if !error.started => return Err(error.message),
        Err(error) => return Ok(unknown_outcome(&error.message, [stdout_ref, stderr_ref])),
    };
    let mut result = Value::object([
        (
            "stdout",
            Value::string(String::from_utf8_lossy(&output.stdout)),
        ),
        (
            "stderr",
            Value::string(String::from_utf8_lossy(&output.stderr)),
        ),
        (
            "exit_code",
            output.exit_code.map_or(Value::Null, Value::number),
        ),
        ("timed_out", Value::Bool(output.timed_out)),
        ("truncated", Value::Bool(output.truncated)),
        ("cancelled", Value::Bool(output.cancelled)),
        ("stdout_bytes", Value::number(output.stdout_bytes)),
        ("stderr_bytes", Value::number(output.stderr_bytes)),
        ("stdout_truncated", Value::Bool(output.stdout_truncated)),
        ("stderr_truncated", Value::Bool(output.stderr_truncated)),
        ("output_limit_bytes", Value::number(OUTPUT_LIMIT)),
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

fn command(bash: &Path, root: &Path, text: &str, temporary: Option<&Path>) -> Command {
    // Git Bash shares its TEMP-derived /tmp mount across live MSYS processes.
    // Initialize it from the inherited environment before exporting a session
    // path that can disappear when that session is deleted.
    #[cfg(windows)]
    let script =
        temporary.map(|_| format!("export TEMP=\"$JECODE_TMP\" TMP=\"$JECODE_TMP\"\n{text}"));
    #[cfg(windows)]
    let text = script.as_deref().unwrap_or(text);
    let mut command = Command::new(bash);
    command.args(["--noprofile", "--norc"]);
    command
        .args(["-c", text])
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
            .env("TMPDIR", &temporary);
        #[cfg(not(windows))]
        command.env("TEMP", &temporary).env("TMP", &temporary);
    }
    command
}

pub fn find_bash() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("JECODE_BASH") {
        if path.is_empty() {
            return Err("JECODE_BASH must name a Bash executable".into());
        }
        return Ok(path.into());
    }
    #[cfg(windows)]
    {
        for variable in [
            "ProgramW6432",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "LOCALAPPDATA",
        ] {
            if let Some(base) = std::env::var_os(variable) {
                for suffix in ["Git/bin/bash.exe", "Programs/Git/bin/bash.exe"] {
                    let path = PathBuf::from(&base).join(suffix);
                    if path.is_file() {
                        return Ok(path);
                    }
                }
            }
        }
        Err("Git Bash was not found. Install Git for Windows or set JECODE_BASH to bash.exe".into())
    }
    #[cfg(not(windows))]
    {
        Ok(PathBuf::from("bash"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;
    use std::time::Instant;

    fn execute(
        bash: &Path,
        root: &Path,
        arguments: &Value,
        cancellation: &Cancellation,
    ) -> Result<Value, String> {
        let outputs =
            crate::output::Store::new(root.join("logs"), crate::redact::Redactor::empty());
        let processes = crate::tools::processes::Registry::default();
        super::execute(
            Context {
                bash,
                root,
                outputs: &outputs,
                temporary: None,
                processes: &processes,
            },
            arguments,
            cancellation,
        )
    }

    fn run(root: &Path, text: &str, timeout: usize) -> Value {
        execute(
            &find_bash().unwrap(),
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
            result.get("stdout").unwrap().as_str().unwrap().len(),
            OUTPUT_LIMIT
        );
        assert_eq!(
            result.get("stderr").unwrap().as_str().unwrap().len(),
            OUTPUT_LIMIT
        );
        assert_eq!(result.get("truncated"), Some(&Value::Bool(true)));
        assert_eq!(result.get("stdout_bytes"), Some(&Value::number(70000)));
        assert_eq!(result.get("stderr_bytes"), Some(&Value::number(70000)));
        assert_eq!(result.get("stdout_truncated"), Some(&Value::Bool(true)));
        assert_eq!(
            result.get("output_limit_bytes"),
            Some(&Value::number(OUTPUT_LIMIT))
        );
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
        let bash = find_bash().unwrap();
        let command = command(&bash, directory.path(), "true", None);
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
            &find_bash().unwrap(),
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
