use super::*;

#[test]
#[ignore = "requires Windows ConPTY for native title lifetime checks"]
fn native_windows_terminal_titles() {
    let directory = Directory::new();
    let executable = std::env::current_exe().unwrap();
    let result = native_script::pty(
        native_pty_source(),
        "Run",
        &[
            &executable.to_string_lossy(),
            &directory.path().to_string_lossy(),
            "title-variants",
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
    println!("{}", String::from_utf8_lossy(&result.stdout));
}

pub(super) fn console() {
    // The encoded long title exceeds the Windows command-line limit. Recovery
    // must carry it without truncation, including after the adapter is killed.
    for previous in [
        String::new(),
        format!("{}🙂", "界".repeat(6000)),
        "Jecode".into(),
    ] {
        console_probe::set_title(&previous);
        assert_eq!(title(), previous);
        for killed in [false, true] {
            let mut terminal = Terminal::open().unwrap();
            assert_eq!(title(), "Jecode", "TUI did not replace the previous title");
            if killed {
                terminal.close_for_fixture();
            }
            drop(terminal);
            assert_eq!(title(), previous, "previous title was not restored");
        }
    }
    for killed in [false, true] {
        console_probe::set_title(CUSTOM_TITLE);
        let mut terminal = Terminal::open().unwrap();
        assert_eq!(title(), "Jecode");
        console_probe::set_title("Changed while Jecode runs β🙂");
        if killed {
            terminal.close_for_fixture();
        }
        drop(terminal);
        assert_eq!(title(), "Changed while Jecode runs β🙂");
    }
    console_probe::set_title(CUSTOM_TITLE);
}
