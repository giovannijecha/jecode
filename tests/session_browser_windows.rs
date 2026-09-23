//! Exercise the actual CLI selector in ConPTY with an isolated home and no login.
#![cfg(windows)]
#[path = "support/conpty.rs"]
mod conpty;
#[path = "support/workspace.rs"]
mod fixture;
#[path = "support/sessions.rs"]
mod sessions;
use std::{
    io::Write,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

#[test]
fn native_console_child() {
    let Some(directory) = std::env::var_os("JECODE_TUI_TEST_DIR") else {
        return;
    };
    let _handles = conpty::bind_test_io();
    let directory = PathBuf::from(directory);
    let launch = || {
        Command::new(env!("CARGO_BIN_EXE_jecode"))
            .arg("--workspace")
            .arg(&directory)
            .arg("--resume")
            .env("USERPROFILE", &directory)
            .env("HOME", &directory)
            .status()
            .unwrap()
    };
    assert!(launch().success(), "Escape should cancel without resuming");
    std::fs::write(directory.join("cancelled"), conpty::snapshot()).unwrap();
    assert_eq!(
        launch().code(),
        Some(2),
        "another owner keeps the chosen session locked"
    );
    std::fs::write(directory.join("selected"), conpty::snapshot()).unwrap();
}

fn wait_for(console: &conpty::Console, predicate: impl Fn(&str) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !predicate(&console.output()) {
        assert!(Instant::now() < deadline, "{}", console.output());
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn cancelling_is_inert_and_selection_leases_only_the_chosen_session() {
    let home = fixture::Fixture::new();
    home.write(".jecode/credentials.json", "untouched legacy fixture");
    sessions::populate(&home.0).unwrap();
    let root = home.0.join(".jecode/v1/sessions");
    let saved = std::fs::read(root.join(format!("{}.json", sessions::NEWER))).unwrap();
    let store = jecode::state::Store::in_home(&home.0)
        .unwrap()
        .directory("sessions")
        .unwrap();
    let _lease = store
        .lock(
            &format!("{}.lock", sessions::NEWER),
            &std::sync::atomic::AtomicBool::new(false),
            Instant::now(),
        )
        .unwrap();
    let mut console = conpty::Console::start(&home.0);
    wait_for(&console, |output| output.contains("Resume >"));
    console.resize(48, 36);
    console.input.write_all(b"\x1b").unwrap();
    wait_for(&console, |_| home.0.join("cancelled").exists());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 3);
    let screen = std::fs::read_to_string(home.0.join("cancelled")).unwrap();
    println!("{screen}");
    assert!(screen.contains("Saved sessions"), "{screen}");
    assert!(
        !screen.contains(sessions::NEWER),
        "IDs should not dominate the terminal"
    );
    wait_for(&console, |output| output.matches("Resume >").count() >= 2);
    console.input.write_all(b"0\r").unwrap();
    wait_for(&console, |output| output.contains("Choose a listed"));
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 3);
    console.input.write_all(b"1\r").unwrap();
    wait_for(&console, |_| home.0.join("selected").exists());
    println!(
        "{}",
        std::fs::read_to_string(home.0.join("selected")).unwrap()
    );
    assert!(root.join(format!("{}.lock", sessions::NEWER)).exists());
    assert!(!root.join(format!("{}.lock", sessions::OLDER)).exists());
    assert_eq!(
        std::fs::read(root.join(format!("{}.json", sessions::NEWER))).unwrap(),
        saved
    );
    assert!(!home.0.join(".jecode/v1/credentials.json").exists());
    drop(console);
}
