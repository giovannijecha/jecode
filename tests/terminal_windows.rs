//! Real Windows ConPTY resize test; snapshots come from the native console buffer.
#![cfg(windows)]
#[path = "support/conpty.rs"]
mod conpty;
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[test]
fn native_console_child() {
    let Some(directory) = std::env::var_os("JECODE_TUI_TEST_DIR") else {
        return;
    };
    let _handles = conpty::bind_test_io();
    let directory = PathBuf::from(directory);
    let stop = Arc::new(AtomicBool::new(false));
    let done = stop.clone();
    let observer = std::thread::spawn(move || {
        let mut previous = String::new();
        while !done.load(Ordering::Relaxed) {
            if let Ok(request) = std::fs::read_to_string(directory.join("request"))
                && !request.is_empty()
                && request != previous
            {
                previous = request;
                let snapshot = conpty::snapshot();
                std::fs::write(
                    directory.join("reply.next"),
                    format!("{previous}\n{snapshot}"),
                )
                .unwrap();
                let _ = std::fs::remove_file(directory.join("reply"));
                std::fs::rename(directory.join("reply.next"), directory.join("reply")).unwrap();
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    let result = jecode::terminal::demo();
    stop.store(true, Ordering::Relaxed);
    observer.join().unwrap();
    result.unwrap();
}

fn snapshot(directory: &Path, counter: &mut usize) -> String {
    *counter += 1;
    let id = counter.to_string();
    std::fs::write(directory.join("request"), &id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(reply) = std::fs::read_to_string(directory.join("reply"))
            && reply.starts_with(&format!("{id}\n"))
        {
            return reply;
        }
        assert!(Instant::now() < deadline, "no console snapshot");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for(directory: &Path, counter: &mut usize, text: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let screen = snapshot(directory, counter);
        if screen.contains(text) {
            return screen;
        }
        assert!(Instant::now() < deadline, "missing {text}: {screen}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn above_composer(screen: &str, text: &str) {
    let rows: Vec<_> = screen.lines().collect();
    let rules: Vec<_> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.contains('─'))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(rules.len(), 2, "{screen}");
    let at = rows.iter().position(|row| row.contains(text)).unwrap();
    assert!(at < rules[0], "{text} entered the composer: {screen}");
}

fn action_previews(console: &mut conpty::Console, directory: &Path, counter: &mut usize) {
    console.input.write_all(b"/edit\r").unwrap();
    let screen = wait_for(directory, counter, "Apply this change?");
    assert!(screen.contains("-       2"), "{screen}");
    assert!(screen.contains("+       3"), "{screen}");
    assert!(screen.contains("› Deny"), "{screen}");
    gap_before(&screen, "pub fn retry_limit");
    for width in [50, 100, 60, 120] {
        console.resize(width, 36);
        std::thread::sleep(Duration::from_millis(150));
        let screen = snapshot(directory, counter);
        assert_eq!(screen.matches("Apply this change?").count(), 1, "{screen}");
        assert_eq!(
            screen.matches("Edit src/settings.rs").count(),
            1,
            "{screen}"
        );
    }
    console.input.write_all(b"\x1b[C\r").unwrap();
    let screen = wait_for(directory, counter, "Simulated edit complete");
    assert_eq!(
        screen.matches("Edit src/settings.rs").count(),
        1,
        "{screen}"
    );
    assert!(!screen.contains("Enter confirm"), "{screen}");
    console.input.write_all(b"/command-error\r").unwrap();
    wait_for(directory, counter, "Run this command?");
    console.input.write_all(b"\x1b[C\r").unwrap();
    let screen = wait_for(directory, counter, "running 2 tests");
    above_composer(&screen, "Running command");
    assert!(!screen.contains("Simulated command complete"), "{screen}");
    console.resize(55, 24);
    std::thread::sleep(Duration::from_millis(150));
    console.resize(120, 36);
    let screen = wait_for(directory, counter, "Simulated command complete");
    assert!(screen.contains("exit 101"), "{screen}");
    assert!(screen.contains("expected: 3, received: 2"), "{screen}");
    assert_eq!(screen.matches("$ cargo test --lib").count(), 1, "{screen}");
    assert_eq!(
        screen.matches("Simulated command complete").count(),
        1,
        "{screen}"
    );
    gap_before(&screen, "running 2 tests");
    gap_before(&screen, "! Simulated command complete");
    assert!(!screen.contains("Running command"), "{screen}");
    console.input.write_all(b"/command\r").unwrap();
    wait_for(directory, counter, "Run this command?");
    console.input.write_all(b"\r").unwrap();
    let screen = wait_for(directory, counter, "Denied");
    assert!(!screen.contains("Enter confirm"), "{screen}");
    assert_eq!(screen.matches("Ask anything").count(), 1, "{screen}");
}

fn gap_before(screen: &str, needle: &str) {
    let rows: Vec<_> = screen.lines().collect();
    let at = rows.iter().position(|row| row.contains(needle)).unwrap();
    assert!(at >= 2, "{screen}");
    assert!(
        rows[at - 1].trim().is_empty(),
        "missing spacing before {needle}: {screen}"
    );
    assert!(
        !rows[at - 2].trim().is_empty(),
        "excess spacing before {needle}: {screen}"
    );
}

#[test]
fn real_windows_resize_keeps_one_composer() {
    let root = std::env::current_dir().unwrap().join("target");
    std::fs::create_dir_all(&root).unwrap();
    let directory = root.join(format!("conpty-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let mut console = conpty::Console::start(&directory);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !console.output().contains("Local demo") {
        assert!(Instant::now() < deadline, "startup: {}", console.output());
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut counter = 0;
    // Inspect during a drag, before the transcript's 100 ms quiet period.
    for width in [80, 62, 95, 70, 50] {
        console.resize(width, 36);
        std::thread::sleep(Duration::from_millis(40));
        let screen = snapshot(&directory, &mut counter);
        let rules: Vec<_> = screen.lines().filter(|line| line.contains('─')).collect();
        assert_eq!(
            rules.len(),
            2,
            "wrapped composer rules during drag: {screen}"
        );
        assert!(
            rules
                .iter()
                .all(|line| line.trim().chars().count() == width as usize - 1),
            "stale composer width during drag: {screen}"
        );
    }
    console.resize(120, 36);
    std::thread::sleep(Duration::from_millis(150));
    console.input.write_all(b"/tools-error\r").unwrap();
    wait_for(&directory, &mut counter, "Exploring workspace");
    for width in [60, 95, 50, 120] {
        console.resize(width, 36);
        std::thread::sleep(Duration::from_millis(150));
        let screen = snapshot(&directory, &mut counter);
        assert_eq!(screen.matches("Exploring workspace").count(), 1, "{screen}");
        above_composer(&screen, "Exploring workspace");
        assert_eq!(screen.matches("Ask anything").count(), 1, "{screen}");
        let header = screen
            .lines()
            .find(|l| l.contains("Exploring workspace"))
            .unwrap();
        assert_eq!(
            header.split("Exploring").next().unwrap().chars().count(),
            2,
            "{screen}"
        );
        assert!(
            matches!(header.chars().next(), Some('\u{2800}'..='\u{28ff}')),
            "{screen}"
        );
    }
    let screen = wait_for(&directory, &mut counter, "completed activity stays");
    assert_eq!(
        screen.matches("Exploration finished with errors").count(),
        1,
        "{screen}"
    );
    assert_eq!(screen.matches("permission denied").count(), 1, "{screen}");
    let header = screen
        .lines()
        .find(|l| l.contains("Exploration finished"))
        .unwrap();
    assert_eq!(
        header.split("Exploration").next().unwrap().chars().count(),
        2,
        "{screen}"
    );
    assert!(
        !screen.contains("Exploring workspace"),
        "active group archived: {screen}"
    );
    console.input.write_all(b"/tools\r").unwrap();
    wait_for(&directory, &mut counter, "Exploring workspace");
    console.input.write_all(b"\x1b").unwrap();
    let screen = wait_for(&directory, &mut counter, "Exploration interrupted");
    assert_eq!(
        screen.matches("Exploration interrupted").count(),
        1,
        "{screen}"
    );
    assert!(
        !screen.contains("Exploring workspace"),
        "cancelled group archived: {screen}"
    );
    action_previews(&mut console, &directory, &mut counter);
    console.input.write_all(b"/long\r").unwrap();
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        let screen = snapshot(&directory, &mut counter);
        if screen.contains("28.") && !screen.contains("Streaming / Esc") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "long response not complete: {screen}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    // Native buffer observations are independent of our direct-VT test surface.
    // ConPTY's resize synchronization protocol is not emulated by that surface.
    for (width, height) in [
        (45, 36),
        (120, 18),
        (45, 40),
        (120, 36),
        (45, 20),
        (120, 36),
    ] {
        console.resize(width, height);
        std::thread::sleep(Duration::from_millis(40));
        let during_drag = snapshot(&directory, &mut counter);
        assert_eq!(
            during_drag
                .lines()
                .filter(|line| line.contains('─'))
                .count(),
            2,
            "wrapped rules after long transcript: {during_drag}"
        );
        std::thread::sleep(Duration::from_millis(140));
        let screen = snapshot(&directory, &mut counter);
        std::fs::write(directory.join(format!("screen-{counter}.txt")), &screen).unwrap();
        assert_eq!(screen.matches("Ask anything").count(), 1, "{screen}");
        assert_eq!(screen.matches("Local demo").count(), 1, "{screen}");
        assert!(screen.contains("28."), "last paragraph lost: {screen}");
        for n in 1..=28 {
            assert!(
                screen.matches(&format!("{n:02}. A useful")).count() <= 1,
                "duplicate paragraph: {screen}"
            );
        }
    }
    for n in 0..80 {
        console.resize(45 + (n * 13 % 75), 18 + (n * 7 % 25));
        std::thread::sleep(Duration::from_millis(5));
    }
    console.resize(45, 40);
    std::thread::sleep(Duration::from_millis(300));
    let screen = snapshot(&directory, &mut counter);
    std::fs::write(directory.join("rapid-screen.txt"), &screen).unwrap();
    assert_eq!(screen.matches("Ask anything").count(), 1, "{screen}");
    assert_eq!(screen.matches("Local demo").count(), 1, "{screen}");
    assert!(screen.contains("28."), "last paragraph lost: {screen}");
    console.input.write_all(b"draft-kept").unwrap();
    std::thread::sleep(Duration::from_millis(150));
    let screen = snapshot(&directory, &mut counter);
    assert_eq!(screen.matches("draft-kept").count(), 1, "{screen}");
    console.input.write_all(b"\x1b").unwrap();
    drop(console);
    // Delete only this test's own directory below the verified workspace target root.
    assert!(
        directory
            .canonicalize()
            .unwrap()
            .starts_with(root.canonicalize().unwrap())
    );
    std::fs::remove_dir_all(directory).unwrap();
}
