use super::{OUTPUT_LIMIT, required_string};
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::process;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub(super) fn execute(
    bash: &Path,
    root: &Path,
    arguments: &Value,
    cancellation: &Cancellation,
    outputs: &crate::output::Store,
    temporary: Option<&Path>,
) -> Result<Value, String> {
    let text = required_string(arguments, "command")?;
    if text.trim().is_empty() {
        return Err("command must not be empty".into());
    }
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
    let mut command = command(bash, root, text);
    command.env_remove("JECODE_TMP");
    if let Some(temporary) = temporary {
        let temporary = crate::scratch::environment_path(temporary);
        for name in ["JECODE_TMP", "TMPDIR", "TEMP", "TMP"] {
            command.env(name, &temporary);
        }
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
        Err(error) => {
            return Ok(Value::object([
                (
                    "error",
                    Value::string(format!(
                        "The command started but its result or output could not be retained: {}. Execution may already have changed the workspace; inspect it before repeating the command.",
                        error.message
                    )),
                ),
                ("outcome", Value::string("unknown")),
                ("stdout_file", Value::string(stdout_ref)),
                ("stderr_file", Value::string(stderr_ref)),
            ]));
        }
    };
    Ok(Value::object([
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
        ("capture", Value::string("tail")),
    ]))
}

fn command(bash: &Path, root: &Path, text: &str) -> Command {
    let mut command = Command::new(bash);
    command.args(["--noprofile", "--norc"]);
    command
        .args(["-c", text])
        .current_dir(root)
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env("TERM", "dumb");
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
        super::execute(
            bash,
            root,
            arguments,
            cancellation,
            &crate::output::Store::new(root.join("logs"), crate::redact::Redactor::empty()),
            None,
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
        assert_eq!(result.get("stdout").unwrap().as_str(), Some("started"));
        assert_eq!(result.get("timed_out"), Some(&Value::Bool(true)));
    }

    #[test]
    fn shell_does_not_inherit_the_api_key_or_startup_script() {
        let directory = Directory::new();
        let bash = find_bash().unwrap();
        let command = command(&bash, directory.path(), "true");
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
