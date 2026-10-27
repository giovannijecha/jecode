use super::*;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Stdio};
use std::sync::mpsc;

#[test]
fn native_bridge_compiles_with_strict_warnings() {
    let mut compiler = Command::new("cc")
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-fsyntax-only",
            "-x",
            "c",
            "-",
        ])
        .arg("-I")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/pty"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    compiler
        .stdin
        .take()
        .unwrap()
        .write_all(SOURCE.as_bytes())
        .unwrap();
    let output = compiler.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

struct Bridge {
    child: Child,
    directory: PathBuf,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(self.directory.join("control"));
        let _ = fs::remove_dir(&self.directory);
    }
}

fn launch(script: &str, path: Option<&str>) -> (Bridge, File, File) {
    let directory =
        std::env::temp_dir().join(format!("jecode-pty-test-{}", crate::sessions::identifier()));
    fs::create_dir(&directory).unwrap();
    let endpoint = directory.join("control");
    let cancellation = Cancellation::default();
    let mut command = Command::new(helper(&cancellation).unwrap());
    command
        .arg(&endpoint)
        .args(["80", "24", "/bin/sh", "-c", script])
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = path {
        command.env("PATH", path);
    }
    let bridge = Bridge {
        child: command.spawn().unwrap(),
        directory,
    };
    let endpoint = endpoint.to_str().unwrap();
    let input = connect(endpoint, &cancellation).unwrap();
    let control = connect_control(endpoint, &cancellation).unwrap();
    (bridge, input, control)
}

fn finished(child: &mut Child) -> std::process::ExitStatus {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "bridge did not exit"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[cfg(target_os = "linux")]
fn linux_bridge_cleanup_works_without_ps_in_path() {
    let (mut bridge, _input, _control) = launch("printf 'READY\\n'; exit 7", Some("/nonexistent"));
    assert_eq!(finished(&mut bridge.child).code(), Some(7));
    let mut output = String::new();
    bridge
        .child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    assert!(output.contains("READY"), "{output}");
}

#[test]
fn immediate_interrupt_waits_for_foreground_process_group() {
    let (mut bridge, _input, mut control) = launch("sleep 30", None);
    control.write_all(&crate::pty::frame(3, &[])).unwrap();
    assert_eq!(finished(&mut bridge.child).code(), Some(130));
}

#[test]
fn resize_and_interrupt_bypass_saturated_input() {
    let script = "stty -echo -icanon; printf 'READY\\n'; trap 'stty size; printf INTERRUPTED\\n; exit 130' INT; while :; do sleep 1; done";
    let (mut bridge, mut input, mut control) = launch(script, None);
    let mut output = bridge.child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut bytes = [0u8; 1024];
        while let Ok(count) = output.read(&mut bytes) {
            if count == 0 || sender.send(bytes[..count].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut seen = Vec::new();
    while !seen.windows(5).any(|bytes| bytes == b"READY") {
        seen.extend(receiver.recv_timeout(Duration::from_secs(5)).unwrap());
    }
    let packet = crate::pty::frame(1, &[b'x'; 4096]);
    let mut saturated = false;
    for _ in 0..512 {
        let mut remaining = packet.as_slice();
        while !remaining.is_empty() {
            match input.write(remaining) {
                Ok(count) => remaining = &remaining[count..],
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    saturated = true;
                    break;
                }
                Err(error) => panic!("input socket failed: {error}"),
            }
        }
        if saturated {
            break;
        }
    }
    assert!(saturated, "input did not encounter backpressure");
    control
        .write_all(&crate::pty::frame(2, &[107, 0, 41, 0]))
        .unwrap();
    control.write_all(&crate::pty::frame(3, &[])).unwrap();
    while !seen.windows(11).any(|bytes| bytes == b"INTERRUPTED") {
        seen.extend(
            receiver
                .recv_timeout(Duration::from_secs(5))
                .unwrap_or_else(|error| {
                    let _ = bridge.child.kill();
                    let _ = bridge.child.wait();
                    let mut diagnostic = String::new();
                    let _ = bridge
                        .child
                        .stderr
                        .take()
                        .unwrap()
                        .read_to_string(&mut diagnostic);
                    panic!(
                        "bridge output ended ({error}): stdout={:?}, stderr={diagnostic:?}",
                        String::from_utf8_lossy(&seen)
                    );
                }),
        );
    }
    assert!(String::from_utf8_lossy(&seen).contains("41 107"));
    assert_eq!(finished(&mut bridge.child).code(), Some(130));
    reader.join().unwrap();
}
