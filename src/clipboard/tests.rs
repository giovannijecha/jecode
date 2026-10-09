use super::{Delivery, Job, MAX_OSC52_INPUT_BYTES, copy, terminal_packet};
use crate::cancel::Cancellation;

fn packet(text: &str) -> Result<Delivery, String> {
    terminal_packet(text, true, &Cancellation::default())
}

#[test]
fn terminal_packet_keeps_utf8_and_newlines() {
    assert_eq!(
        packet("é🙂\r\nline\n"),
        Ok(Delivery::Terminal(
            "\x1b]52;c;w6nwn5mCDQpsaW5lCg==\x07".into()
        ))
    );
}

#[test]
fn invalid_content_never_reaches_a_native_command() {
    assert_eq!(
        copy("", true, &Cancellation::default()),
        Err("There is no text to copy".into())
    );
    assert_eq!(
        copy("before\0after", true, &Cancellation::default()),
        Err("Text containing a NUL character cannot be copied".into())
    );
}

#[test]
fn cancelled_request_does_not_prepare_delivery() {
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert_eq!(
        copy("content", true, &cancellation),
        Err("Operation cancelled".into())
    );
}

#[test]
fn terminal_packet_requires_a_terminal_and_has_a_raw_byte_limit() {
    assert!(terminal_packet("content", false, &Cancellation::default()).is_err());
    let max = "x".repeat(MAX_OSC52_INPUT_BYTES);
    assert!(matches!(packet(&max), Ok(Delivery::Terminal(_))));
    assert!(packet(&(max + "x")).is_err());
    let multibyte = "é".repeat(MAX_OSC52_INPUT_BYTES / 2 + 1);
    assert!(packet(&multibyte).is_err());
}

#[test]
fn fixture_is_complete_without_native_io() {
    let job = Job::fixture(Ok(Delivery::Confirmed));
    assert!(job.finished());
    assert_eq!(job.finish(), Ok(Delivery::Confirmed));
}

#[test]
fn worker_reports_validation_error_without_native_io() {
    let job = Job::start(String::new(), false);
    assert_eq!(job.finish(), Err("There is no text to copy".into()));
}

#[cfg(windows)]
#[test]
fn windows_command_does_not_contain_user_text() {
    let command = super::windows_command();
    let args: Vec<_> = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    assert_eq!(command.get_program(), "powershell.exe");
    assert!(args.iter().any(|arg| arg == "-NoProfile"));
    assert!(args.iter().any(|arg| arg.contains("Set-Clipboard")));
}

#[cfg(windows)]
#[test]
fn windows_script_receives_exact_utf8_without_touching_clipboard() {
    use std::process::Command;

    // Shadow the cmdlet inside this process. Any conversion, BOM stripping, or
    // newline change makes the fake Set-Clipboard fail before native mutation.
    let script = format!(
        "$script:expected = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('77u/w6nwn5mCDQpsYXN0Cg==')); function Set-Clipboard {{ param([string]$Value) if ($Value -cne $script:expected) {{ throw 'Unexpected clipboard text' }} }}; {}",
        super::WINDOWS_SCRIPT
    );
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Sta",
        "-Command",
        &script,
    ]);
    let copied = super::native_copy(command, "\u{feff}é🙂\r\nlast\n", &Cancellation::default());
    assert_eq!(copied, Ok(Delivery::Confirmed));
}
