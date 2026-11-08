use super::*;

fn probe_directory() -> PathBuf {
    use std::os::unix::{ffi::OsStrExt, fs::DirBuilderExt};
    let directory = PathBuf::from("/tmp").join(format!("jc-pty-{}", crate::sessions::identifier()));
    assert!(directory.join("control").as_os_str().as_bytes().len() < 96);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    directory
}

fn next_gate_event(
    listener: &std::os::unix::net::UnixListener,
) -> ([u8; 3], std::os::unix::net::UnixStream) {
    let started = std::time::Instant::now();
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_nonblocking(true).unwrap();
                let mut event = [0; 3];
                let mut received = 0;
                while received < event.len() {
                    match stream.read(&mut event[received..]) {
                        Ok(0) => panic!("watcher/resize gate closed after {received} bytes"),
                        Ok(count) => received += count,
                        Err(error)
                            if matches!(
                                error.kind(),
                                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                            ) =>
                        {
                            assert!(
                                started.elapsed() < std::time::Duration::from_secs(5),
                                "watcher/resize gate event was incomplete"
                            );
                            std::thread::yield_now();
                        }
                        Err(error) => panic!("watcher/resize gate read failed: {error}"),
                    }
                }
                return (event, stream);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                assert!(
                    started.elapsed() < std::time::Duration::from_secs(5),
                    "watcher/resize gate did not report within five seconds"
                );
                std::thread::yield_now();
            }
            Err(error) => panic!("watcher/resize gate failed: {error}"),
        }
    }
}

fn run_post_exit_resize(mode: &str, forced_error: bool, exit_code: i32) {
    use std::os::unix::net::UnixListener;
    let probe = Probe::compile();
    let directory = probe_directory();
    let gate = directory.join("gate");
    let listener = UnixListener::bind(&gate).unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = directory.join("control");
    let script = format!("exit {exit_code}");
    let mut command = Command::new(&probe.executable);
    command
        .arg(&endpoint)
        .args(["80", "24", "/bin/sh", "-c", &script])
        .env("JECODE_PTY_TEST_INJECT", mode)
        .env("JECODE_PTY_STATUS_GATE", &gate)
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut bridge = Bridge {
        child: command.spawn().unwrap(),
        directory,
    };
    let endpoint = endpoint.to_str().unwrap();
    let cancellation = Cancellation::default();
    let _input = connect(endpoint, &cancellation).unwrap();
    let mut control = connect_control(endpoint, &cancellation).unwrap();

    let (exit, _) = next_gate_event(&listener);
    assert_eq!(exit, [b'E', 0, 0], "target exit was not observed");
    let (marker, mut exit_gate) = next_gate_event(&listener);
    assert_eq!(marker, [b'M', 0, 0], "early exit marker was not reached");
    control
        .write_all(&crate::pty::frame(2, &[81, 0, 25, 0]))
        .unwrap();
    let (resize, _) = next_gate_event(&listener);
    assert_eq!(resize[0], b'R', "resize ioctl was not reached");
    if forced_error {
        assert_eq!(resize, [b'R', 1, 9], "forced EBADF was not observed");
    } else {
        assert!(
            resize[1] == 0 || resize == [b'R', 1, 9],
            "unexpected resize result: {resize:?}"
        );
    }
    exit_gate.write_all(b"g").unwrap();
    let (cleaned, mut status_gate) = next_gate_event(&listener);
    assert_eq!(cleaned, [b'S', 0, 0], "watcher cleanup did not finish");
    assert!(
        bridge.child.try_wait().unwrap().is_none(),
        "bridge exited before cleanup status"
    );
    status_gate.write_all(b"g").unwrap();
    let status = finished(&mut bridge.child);
    let mut diagnostics = String::new();
    bridge
        .child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut diagnostics)
        .unwrap();
    assert_eq!(
        status.code(),
        Some(exit_code),
        "bridge exited after resize: {diagnostics}"
    );
}

#[test]
fn resize_after_target_exit_before_watcher_status() {
    run_post_exit_resize("exit_resize", false, 0);
}

#[test]
fn revoked_resize_after_exit_waits_for_cleanup() {
    run_post_exit_resize("exit_resize_error", true, 0);
}

#[test]
fn revoked_resize_preserves_child_exit_code() {
    run_post_exit_resize("exit_resize_error", true, 7);
}

#[test]
fn lost_query_write_uses_authoritative_watcher_status() {
    run_post_exit_resize("exit_resize_epipe", true, 0);
}

#[test]
fn query_rechecks_exit_after_an_empty_waitid_sample() {
    let probe = Probe::compile();
    let directory = probe_directory();
    let endpoint = directory.join("control");
    let mut command = Command::new(&probe.executable);
    command
        .arg(&endpoint)
        .args(["80", "24", "/bin/sh", "-c", "printf READY; sleep 30"])
        .env("JECODE_PTY_TEST_INJECT", "query_exit_race")
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut bridge = Bridge {
        child: command.spawn().unwrap(),
        directory,
    };
    let endpoint = endpoint.to_str().unwrap();
    let cancellation = Cancellation::default();
    let _input = connect(endpoint, &cancellation).unwrap();
    let mut control = connect_control(endpoint, &cancellation).unwrap();
    let (receiver, reader) = output_events(&mut bridge);
    let mut seen = Vec::new();
    until(&receiver, &mut seen, b"READY");
    control
        .write_all(&crate::pty::frame(2, &[81, 0, 25, 0]))
        .unwrap();
    let status = finished(&mut bridge.child);
    let mut diagnostics = String::new();
    bridge
        .child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut diagnostics)
        .unwrap();
    reader.join().unwrap();
    assert_eq!(status.code(), Some(137), "{diagnostics}");
}

#[test]
fn resize_failure_while_target_runs_remains_fatal() {
    let probe = Probe::compile();
    let directory = probe_directory();
    let endpoint = directory.join("control");
    let mut command = Command::new(&probe.executable);
    command
        .arg(&endpoint)
        .args(["80", "24", "/bin/sh", "-c", "printf READY; sleep 30"])
        .env("JECODE_PTY_TEST_INJECT", "live_resize_error")
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut bridge = Bridge {
        child: command.spawn().unwrap(),
        directory,
    };
    let endpoint = endpoint.to_str().unwrap();
    let cancellation = Cancellation::default();
    let _input = connect(endpoint, &cancellation).unwrap();
    let mut control = connect_control(endpoint, &cancellation).unwrap();
    let (receiver, reader) = output_events(&mut bridge);
    let mut seen = Vec::new();
    until(&receiver, &mut seen, b"READY");
    control
        .write_all(&crate::pty::frame(2, &[81, 0, 25, 0]))
        .unwrap();
    let status = finished(&mut bridge.child);
    let mut diagnostics = String::new();
    bridge
        .child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut diagnostics)
        .unwrap();
    reader.join().unwrap();
    assert_eq!(status.code(), Some(125), "{diagnostics}");
    assert!(
        diagnostics.contains("Unix PTY resize failed: Bad file descriptor"),
        "{diagnostics}"
    );
}

#[test]
fn invalid_control_frame_remains_fatal() {
    let probe = Probe::compile();
    let directory = probe_directory();
    let endpoint = directory.join("control");
    let mut command = Command::new(&probe.executable);
    command
        .arg(&endpoint)
        .args(["80", "24", "/bin/sh", "-c", "sleep 30"])
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut bridge = Bridge {
        child: command.spawn().unwrap(),
        directory,
    };
    let endpoint = endpoint.to_str().unwrap();
    let cancellation = Cancellation::default();
    let _input = connect(endpoint, &cancellation).unwrap();
    let mut control = connect_control(endpoint, &cancellation).unwrap();
    control.write_all(&[9, 0, 0, 0, 0]).unwrap();
    let status = finished(&mut bridge.child);
    let mut diagnostics = String::new();
    bridge
        .child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut diagnostics)
        .unwrap();
    assert_eq!(status.code(), Some(125), "{diagnostics}");
    assert!(
        diagnostics.contains("Unix PTY control frame was invalid"),
        "{diagnostics}"
    );
}
