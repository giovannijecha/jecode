//! Native ConPTY exercise of the Rust terminal panel and direct PTY routing.
use super::*;

#[test]
#[ignore = "requires Windows ConPTY and PowerShell for native direct input"]
fn native_windows_direct_input() {
    let directory = Directory::new();
    let executable = std::env::current_exe().unwrap();
    let result = native_script::pty(
        native_pty_source(),
        "Run",
        &[
            &executable.to_string_lossy(),
            &directory.path().to_string_lossy(),
            "direct-input",
        ],
        &directory,
        Duration::from_secs(60),
        false,
    );
    assert_eq!(
        result.exit_code,
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&result.stderr),
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(!result.timed_out);
}
