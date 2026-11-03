use super::*;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Stdio;
use std::time::Instant;

const TREE_CHILD: &str = "process::duplex::tests::duplex_tree_child";
const TREE_GRANDCHILD: &str = "process::duplex::tests::duplex_tree_grandchild";
const HEARTBEAT: &str = "JECODE_DUPLEX_HEARTBEAT";

#[test]
fn duplex_echo_child() {
    if std::env::var_os("JECODE_DUPLEX_CHILD").is_none() {
        return;
    }
    let input = std::io::stdin();
    let mut output = std::io::stdout().lock();
    for line in input.lock().lines() {
        let line = line.unwrap();
        if line == "exit" {
            std::process::exit(0);
        }
        writeln!(output, "echo:{line}").unwrap();
        output.flush().unwrap();
    }
    std::process::exit(0);
}

fn child() -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "process::duplex::tests::duplex_echo_child",
            "--nocapture",
        ])
        .env("JECODE_DUPLEX_CHILD", "1");
    command
}

#[test]
fn duplex_tree_child() {
    if std::env::var_os("JECODE_DUPLEX_TREE_CHILD").is_none() {
        return;
    }
    let mut grandchild = Command::new(std::env::current_exe().unwrap());
    grandchild
        .args(["--exact", TREE_GRANDCHILD, "--nocapture"])
        .env("JECODE_DUPLEX_TREE_GRANDCHILD", "1")
        .env(HEARTBEAT, std::env::var_os(HEARTBEAT).unwrap())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut grandchild = grandchild.spawn().unwrap();
    std::thread::spawn(move || {
        let _ = grandchild.wait();
    });
    loop {
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn duplex_tree_grandchild() {
    if std::env::var_os("JECODE_DUPLEX_TREE_GRANDCHILD").is_none() {
        return;
    }
    let path = std::env::var_os(HEARTBEAT).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(12) {
        if file.write_all(b".").is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn tree_child(heartbeat: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", TREE_CHILD, "--nocapture"])
        .env("JECODE_DUPLEX_TREE_CHILD", "1")
        .env(HEARTBEAT, heartbeat);
    command
}

fn heartbeat_len(path: &Path) -> u64 {
    match std::fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => panic!("Could not inspect heartbeat: {error}"),
    }
}

fn wait_for_heartbeat(path: &Path) {
    let started = Instant::now();
    while heartbeat_len(path) < 3 {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "Grandchild did not start"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn assert_heartbeat_stopped(path: &Path) {
    // Allow a write already in progress to finish before checking stability.
    std::thread::sleep(Duration::from_millis(150));
    let length = heartbeat_len(path);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        heartbeat_len(path),
        length,
        "Grandchild kept writing after stop"
    );
}

#[test]
fn persistent_input_exchanges_multiple_messages_and_closes() {
    let mut process = Duplex::spawn_cancellable(&mut child(), &Cancellation::default()).unwrap();
    assert_ne!(process.id(), 0);
    let mut stdout = BufReader::new(process.stdout.take().unwrap());
    for text in ["first", "second 🦀"] {
        writeln!(process.stdin.as_mut().unwrap(), "{text}").unwrap();
        process.stdin.as_mut().unwrap().flush().unwrap();
        let mut line = String::new();
        loop {
            assert_ne!(stdout.read_line(&mut line).unwrap(), 0);
            if line.contains("echo:") {
                break;
            }
            line.clear();
        }
        assert_eq!(line.trim_end(), format!("echo:{text}"));
    }
    assert!(!process.wait_timeout(Duration::from_millis(10)).unwrap());
    writeln!(process.stdin.as_mut().unwrap(), "exit").unwrap();
    process.stdin.as_mut().unwrap().flush().unwrap();
    assert!(process.wait_timeout(Duration::from_secs(5)).unwrap());
}

#[test]
fn cancellation_before_launch_creates_no_process() {
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(Duplex::spawn_cancellable(&mut child(), &cancel).is_err());
}

#[test]
fn forced_stop_ends_an_idle_server() {
    let mut process = Duplex::spawn_cancellable(&mut child(), &Cancellation::default()).unwrap();
    process.kill_tree().unwrap();
    assert!(process.wait_timeout(Duration::from_secs(2)).unwrap());
}

#[test]
fn kill_tree_stops_a_running_grandchild() {
    let directory = crate::test_support::Directory::new();
    let heartbeat = directory.path().join("kill-tree-heartbeat");
    let mut process =
        Duplex::spawn_cancellable(&mut tree_child(&heartbeat), &Cancellation::default()).unwrap();
    wait_for_heartbeat(&heartbeat);
    process.kill_tree().unwrap();
    assert!(process.wait_timeout(Duration::from_secs(2)).unwrap());
    assert_heartbeat_stopped(&heartbeat);
}

#[test]
fn drop_stops_a_running_grandchild() {
    let directory = crate::test_support::Directory::new();
    let heartbeat = directory.path().join("drop-heartbeat");
    let process =
        Duplex::spawn_cancellable(&mut tree_child(&heartbeat), &Cancellation::default()).unwrap();
    wait_for_heartbeat(&heartbeat);
    drop(process);
    assert_heartbeat_stopped(&heartbeat);
}
