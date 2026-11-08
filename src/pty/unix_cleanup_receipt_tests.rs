use super::*;
use crate::{output, process, redact::Redactor, test_support::Directory};
use std::os::unix::net::{UnixListener, UnixStream};

struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        let path = PathBuf::from("/tmp").join(format!("jc-rcpt-{}", crate::sessions::identifier()));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self { path }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.path.join("control"));
        let _ = fs::remove_file(self.path.join("gate"));
        let _ = fs::remove_dir(&self.path);
    }
}

fn receipt_gate(listener: &UnixListener) -> ([u8; 3], UnixStream) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_nonblocking(true).unwrap();
                let mut event = [0; 3];
                let mut count = 0;
                while count < event.len() {
                    match stream.read(&mut event[count..]) {
                        Ok(0) => panic!("cleanup receipt gate closed early"),
                        Ok(read) => count += read,
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "cleanup receipt event timed out");
                            thread::yield_now();
                        }
                        Err(error) => panic!("cleanup receipt gate read failed: {error}"),
                    }
                }
                return (event, stream);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "cleanup receipt gate timed out");
                thread::yield_now();
            }
            Err(error) => panic!("cleanup receipt gate accept failed: {error}"),
        }
    }
}

fn run_receipt(mode: &str) {
    let probe = Probe::compile();
    let fixture = Fixture::new();
    let endpoint = fixture.path.join("control");
    let gate = fixture.path.join("gate");
    let listener = UnixListener::bind(&gate).unwrap();
    listener.set_nonblocking(true).unwrap();
    let directory = Directory::new();
    let store = output::Store::new(directory.path().join("output"), Redactor::empty());
    let logs = store.create().unwrap();
    let stdout = store.resolve(&logs.stdout_ref).unwrap();
    let mut command = Command::new(&probe.executable);
    command
        .arg(&endpoint)
        .args(["80", "24", "/bin/sh", "-c", "printf READY; sleep 30"])
        .env("JECODE_PTY_TEST_INJECT", mode)
        .env("JECODE_PTY_STATUS_GATE", &gate);
    let cancellation = Cancellation::default();
    let mut started =
        process::start_logged(&mut command, &cancellation, [logs.stdout, logs.stderr]).unwrap();
    let endpoint = endpoint.to_str().unwrap();
    let _input = connect(endpoint, &cancellation).unwrap();
    let control = connect_control(endpoint, &cancellation).unwrap();
    started.expect_pty_cleanup_receipt(control.try_clone().unwrap());
    let ready_deadline = Instant::now() + Duration::from_secs(5);
    while !fs::read_to_string(&stdout).unwrap().contains("READY") {
        assert!(Instant::now() < ready_deadline, "PTY target did not start");
        thread::sleep(Duration::from_millis(10));
    }
    let stop = cancellation.clone();
    let (sender, receiver) = mpsc::channel();
    let monitor = thread::spawn(move || {
        let result = started.supervise(None, &stop, &mut |_| Ok(()));
        let _ = sender.send(result);
    });
    cancellation.cancel();
    let (event, mut release) = receipt_gate(&listener);
    assert_eq!(
        event,
        [b'C', u8::from(mode != "receipt_fail"), 0],
        "watcher cleanup did not reach expected receipt"
    );
    assert!(
        matches!(
            receiver.recv_timeout(Duration::from_millis(600)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ),
        "PTY stop resolved before the watcher receipt"
    );
    release.write_all(b"g").unwrap();
    let result = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    monitor.join().unwrap();
    assert!(fs::read_to_string(stdout).unwrap().contains("READY"));
    match mode {
        "receipt_gate" => {
            let output = result.unwrap();
            assert!(
                output.cancelled,
                "PTY stop did not produce a stopped outcome"
            );
        }
        "receipt_fail" => {
            let error = result.err().unwrap().message;
            assert!(
                error.contains("could not confirm session cleanup"),
                "{error}"
            );
        }
        "receipt_missing" => {
            let error = result.err().unwrap().message;
            assert!(error.contains("cleanup receipt was missing"), "{error}");
        }
        _ => unreachable!(),
    }
}

#[test]
fn watcher_receipt_after_legacy_deadline_stops_owned_pty() {
    run_receipt("receipt_gate");
}

#[test]
fn failed_watcher_receipt_preserves_unknown_outcome() {
    run_receipt("receipt_fail");
}

#[test]
fn missing_watcher_receipt_preserves_unknown_outcome() {
    run_receipt("receipt_missing");
}
