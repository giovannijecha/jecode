//! Run the complete native fixture script without placing its C# source on
//! the Windows command line, whose 32,767-character limit depends on paths.

use crate::{cancel::Cancellation, process, test_support::Directory};
use std::{path::Path, process::Command, time::Duration};

const BOOTSTRAP: &str = "$ErrorActionPreference = 'Stop'; $reader = [IO.StreamReader]::new([Console]::OpenStandardInput(), [Text.UTF8Encoding]::new($false, $true), $false); try { & ([ScriptBlock]::Create($reader.ReadToEnd())) } finally { $reader.Dispose() }";

pub(super) fn pty(
    source: String,
    entry: &str,
    arguments: &[&str],
    directory: &Directory,
    timeout: Duration,
    strip_model: bool,
) -> process::Output {
    let arguments = arguments
        .iter()
        .map(|argument| format!("'{}'", argument.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    let script = format!(
        "Add-Type -TypeDefinition @'\n{source}\n'@\n[JecodeTestPty]::{entry}({arguments})\n"
    );
    run(script, directory.path(), timeout, strip_model)
}

fn run(script: String, directory: &Path, timeout: Duration, strip_model: bool) -> process::Output {
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            BOOTSTRAP,
        ])
        .env("TEMP", directory)
        .env("TMP", directory)
        .env_remove("OPENROUTER_API_KEY");
    if strip_model {
        command.env_remove("OPENROUTER_MODEL");
    }
    process::run_observed(
        &mut command,
        Some(script.into_bytes()),
        timeout,
        64 * 1024,
        &Cancellation::default(),
        None,
        &mut |_| Ok(()),
    )
    .unwrap()
}

#[test]
fn a_unicode_script_beyond_the_windows_command_line_limit_runs_from_stdin() {
    let directory = Directory::new();
    let decoded = directory.path().join("decoded.txt");
    let script = format!(
        "[IO.File]::WriteAllText('{}', '雪🙂', [Text.UTF8Encoding]::new($false))\nWrite-Output 'NATIVE_STDIN_OK'\n#{}",
        decoded.to_string_lossy().replace('\'', "''"),
        "雪".repeat(33_000)
    );
    assert!(script.encode_utf16().count() > 32_767);
    let inline = process::run_observed(
        Command::new("powershell.exe").args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ]),
        None,
        Duration::from_secs(15),
        1024,
        &Cancellation::default(),
        None,
        &mut |_| Ok(()),
    );
    match inline {
        Err(error) => assert!(error.contains("206"), "{error}"),
        Ok(output) => panic!("inline script launched: {:?}", output.exit_code),
    }
    let output = run(script, directory.path(), Duration::from_secs(15), false);
    assert_eq!(
        output.exit_code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "NATIVE_STDIN_OK"
    );
    assert_eq!(std::fs::read_to_string(decoded).unwrap(), "雪🙂");
}

#[test]
fn an_exception_in_the_piped_script_fails_the_native_host() {
    let directory = Directory::new();
    let output = run(
        "throw 'NATIVE_SCRIPT_FAILURE'".into(),
        directory.path(),
        Duration::from_secs(15),
        false,
    );
    assert_ne!(output.exit_code, Some(0));
    assert!(String::from_utf8_lossy(&output.stderr).contains("NATIVE_SCRIPT_FAILURE"));
}
