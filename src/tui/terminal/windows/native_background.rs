//! Native ConPTY coverage for the per-renderer fallback and the Windows
//! main-screen background hint used when OSC 11 is unanswered.

use super::*;

fn run_host(scenario: &str) {
    let directory = Directory::new();
    let executable = std::env::current_exe().unwrap();
    let result = native_script::pty(
        native_pty_source(),
        "Run",
        &[
            &executable.to_string_lossy(),
            &directory.path().to_string_lossy(),
            scenario,
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

#[test]
#[ignore = "requires Windows ConPTY for native light fallback rendering"]
fn native_windows_light_background() {
    run_host("background-light");
}

#[test]
#[ignore = "requires Windows ConPTY for native console background detection"]
fn native_windows_background_detection() {
    run_host("background-native");
}
