//! Real controller + v2 persistence through a native ConPTY and fake home.
use super::menu_native_tests::conpty;
use super::*;
use std::{
    io::Write,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[test]
fn native_real_session_child() {
    let Some(root) = std::env::var_os("JECODE_TUI_TEST_DIR") else {
        return;
    };
    let _handles = conpty::bind_test_io();
    let root = std::path::PathBuf::from(root);
    let home = root.join("home");
    let directory = root.join("workspace");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&directory).unwrap();
    assert_eq!(
        std::env::var_os("USERPROFILE"),
        Some(home.as_os_str().to_owned())
    );
    let session = crate::session::tests::persisted_terminal_fixture(&home, &directory);
    let directory = crate::session::scope::Directory::open(&directory).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let done = stop.clone();
    let observer = std::thread::spawn(move || {
        let mut previous = String::new();
        while !done.load(Ordering::Relaxed) {
            if let Ok(request) = std::fs::read_to_string(root.join("request"))
                && !request.is_empty()
                && request != previous
            {
                previous = request;
                let screen = conpty::snapshot();
                std::fs::write(root.join("reply.next"), format!("{previous}\n{screen}")).unwrap();
                let _ = std::fs::remove_file(root.join("reply"));
                std::fs::rename(root.join("reply.next"), root.join("reply")).unwrap();
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    let result = run_once(navigation::Start {
        selected: Some(crate::session::Model::Luna),
        directory: Some(directory),
        workspace: None,
        saved: None,
        prepared: Some(session),
        carried: None,
        pending: None,
    });
    stop.store(true, Ordering::Relaxed);
    observer.join().unwrap();
    assert!(result.unwrap().is_none());
}

fn snapshot(directory: &Path, sequence: &mut usize) -> String {
    *sequence += 1;
    let id = sequence.to_string();
    std::fs::write(directory.join("request"), &id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(reply) = std::fs::read_to_string(directory.join("reply"))
            && reply.starts_with(&format!("{id}\n"))
        {
            return reply;
        }
        assert!(
            Instant::now() < deadline,
            "ConPTY snapshot {id} unavailable"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for(directory: &Path, sequence: &mut usize, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let screen = snapshot(directory, sequence);
        if screen.contains(needle) {
            return screen;
        }
        assert!(Instant::now() < deadline, "missing {needle}: {screen}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn real_controller_turn_persists_and_presentation_replay_is_effect_free() {
    let target = std::env::current_dir().unwrap().join("target");
    std::fs::create_dir_all(&target).unwrap();
    let directory = target.join(format!("conpty-real-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let home = directory.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut console = conpty::Console::start_named_in_home(
        &directory,
        "terminal::real_conpty_tests::native_real_session_child",
        Some(&home),
    );
    let mut sequence = 0;
    wait_for(&directory, &mut sequence, "Ask anything");
    console.input.write_all(b"hello\r").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let screen = loop {
        let screen = snapshot(&directory, &mut sequence);
        if screen.contains("partial completed") && !screen.contains("Streaming ·") {
            break screen;
        }
        assert!(Instant::now() < deadline, "turn did not settle: {screen}");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(screen.contains("hello"), "{screen}");
    assert!(screen.contains("✓ Complete"), "{screen}");
    let evidence = target.join("tui-evidence");
    std::fs::create_dir_all(&evidence).unwrap();
    std::fs::write(evidence.join("real-controller-conpty.txt"), &screen).unwrap();
    let logs = directory.join("home/.jecode/v1/sessions-v2");
    let file = std::fs::read_dir(logs)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|extension| extension == "log"))
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let canonical = std::fs::read(&file).unwrap();
    assert!(!canonical.is_empty());
    console.input.write_all(b"\x0f").unwrap();
    console.resize(80, 24);
    std::thread::sleep(Duration::from_millis(180));
    let screen = snapshot(&directory, &mut sequence);
    assert!(screen.contains("partial completed"), "{screen}");
    assert_eq!(std::fs::read(&file).unwrap(), canonical);
    console.input.write_all(&[17]).unwrap();
    drop(console);
    assert!(
        directory
            .canonicalize()
            .unwrap()
            .starts_with(target.canonicalize().unwrap())
    );
    std::fs::remove_dir_all(directory).unwrap();
}
