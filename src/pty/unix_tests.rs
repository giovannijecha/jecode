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

fn input_frame(generation: u64, bytes: &[u8]) -> Vec<u8> {
    crate::pty::frame(1, &[generation.to_le_bytes().as_slice(), bytes].concat())
}

fn interrupt_frame(generation: u64) -> Vec<u8> {
    crate::pty::frame(3, &generation.to_le_bytes())
}

fn output_events(bridge: &mut Bridge) -> (mpsc::Receiver<Vec<u8>>, thread::JoinHandle<()>) {
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
    (receiver, reader)
}

fn until(receiver: &mpsc::Receiver<Vec<u8>>, seen: &mut Vec<u8>, marker: &[u8]) {
    while !seen.windows(marker.len()).any(|bytes| bytes == marker) {
        seen.extend(receiver.recv_timeout(Duration::from_secs(5)).unwrap());
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
    control.write_all(&interrupt_frame(1)).unwrap();
    assert_eq!(finished(&mut bridge.child).code(), Some(130));
}

#[test]
fn interrupt_flushes_canonical_typeahead() {
    let script = "stty -echo; trap 'kill \"$sleeper\" 2>/dev/null; wait \"$sleeper\" 2>/dev/null; printf INTERRUPTED\\n' INT; sleep 30 & sleeper=$!; printf 'READY\\n'; wait \"$sleeper\"; IFS= read -r line; printf 'LINE:%s:END\\n' \"$line\"";
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
    input.write_all(&input_frame(0, b"OLD\n")).unwrap();
    control.write_all(&interrupt_frame(1)).unwrap();
    while !seen.windows(11).any(|bytes| bytes == b"INTERRUPTED") {
        seen.extend(receiver.recv_timeout(Duration::from_secs(5)).unwrap());
    }
    input.write_all(&input_frame(1, b"NEW\n")).unwrap();
    while !seen.windows(4).any(|bytes| bytes == b":END") {
        seen.extend(receiver.recv_timeout(Duration::from_secs(5)).unwrap());
    }
    assert!(
        String::from_utf8_lossy(&seen).contains("LINE:NEW"),
        "{seen:?}"
    );
    assert_eq!(finished(&mut bridge.child).code(), Some(0));
    reader.join().unwrap();
}

#[test]
fn interrupt_discards_delayed_preinterrupt_input() {
    let script = "stty -echo; trap 'kill \"$sleeper\" 2>/dev/null; wait \"$sleeper\" 2>/dev/null; printf INTERRUPTED\\n' INT; sleep 30 & sleeper=$!; printf 'READY\\n'; wait \"$sleeper\"; IFS= read -r line; printf 'LINE:%s:END\\n' \"$line\"";
    let (mut bridge, mut input, mut control) = launch(script, None);
    let (receiver, reader) = output_events(&mut bridge);
    let mut seen = Vec::new();
    until(&receiver, &mut seen, b"READY");
    control.write_all(&interrupt_frame(1)).unwrap();
    until(&receiver, &mut seen, b"INTERRUPTED");
    input.write_all(&input_frame(0, b"OLD\n")).unwrap();
    input.write_all(&input_frame(1, b"NEW\n")).unwrap();
    until(&receiver, &mut seen, b":END");
    assert!(
        String::from_utf8_lossy(&seen).contains("LINE:NEW:END"),
        "{seen:?}"
    );
    assert_eq!(finished(&mut bridge.child).code(), Some(0));
    reader.join().unwrap();
}

#[test]
fn input_from_after_interrupt_waits_for_its_control_frame() {
    let script = "stty -echo; trap 'kill \"$sleeper\" 2>/dev/null; wait \"$sleeper\" 2>/dev/null; printf INTERRUPTED\\n' INT; sleep 30 & sleeper=$!; printf 'READY\\n'; wait \"$sleeper\"; IFS= read -r line; printf 'LINE:%s:END\\n' \"$line\"";
    let (mut bridge, mut input, mut control) = launch(script, None);
    let (receiver, reader) = output_events(&mut bridge);
    let mut seen = Vec::new();
    until(&receiver, &mut seen, b"READY");
    input.write_all(&input_frame(1, b"FUTURE\n")).unwrap();
    let packet = input_frame(1, &[b'x'; 4096]);
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
    control.write_all(&interrupt_frame(1)).unwrap();
    until(&receiver, &mut seen, b":END");
    assert!(
        String::from_utf8_lossy(&seen).contains("INTERRUPTED"),
        "{seen:?}"
    );
    assert!(
        String::from_utf8_lossy(&seen).contains("LINE:FUTURE:END"),
        "{seen:?}"
    );
    assert_eq!(finished(&mut bridge.child).code(), Some(0));
    reader.join().unwrap();
}

#[test]
fn interrupt_preserves_input_with_noflsh() {
    let script = "stty -echo noflsh; trap 'kill \"$sleeper\" 2>/dev/null; wait \"$sleeper\" 2>/dev/null; printf INTERRUPTED\\n' INT; sleep 30 & sleeper=$!; printf 'READY\\n'; wait \"$sleeper\"; IFS= read -r line; printf 'LINE:%s:END\\n' \"$line\"";
    let (mut bridge, mut input, mut control) = launch(script, None);
    let (receiver, reader) = output_events(&mut bridge);
    let mut seen = Vec::new();
    until(&receiver, &mut seen, b"READY");
    input.write_all(&input_frame(0, b"OLD\n")).unwrap();
    control.write_all(&interrupt_frame(1)).unwrap();
    until(&receiver, &mut seen, b":END");
    assert!(
        String::from_utf8_lossy(&seen).contains("INTERRUPTED"),
        "{seen:?}"
    );
    assert!(
        String::from_utf8_lossy(&seen).contains("LINE:OLD:END"),
        "{seen:?}"
    );
    assert_eq!(finished(&mut bridge.child).code(), Some(0));
    reader.join().unwrap();
}

#[test]
fn interrupt_in_raw_mode_delivers_literal_etx() {
    let script = "stty raw -echo; printf 'READY\\n'; od -An -tu1 -N 1; printf 'DONE\\n'";
    let (mut bridge, _input, mut control) = launch(script, None);
    let (receiver, reader) = output_events(&mut bridge);
    let mut seen = Vec::new();
    until(&receiver, &mut seen, b"READY");
    control.write_all(&interrupt_frame(1)).unwrap();
    until(&receiver, &mut seen, b"DONE");
    assert!(String::from_utf8_lossy(&seen).contains(" 3\n"), "{seen:?}");
    assert_eq!(finished(&mut bridge.child).code(), Some(0));
    reader.join().unwrap();
}

#[test]
#[cfg(target_os = "linux")]
fn blocked_raw_interrupts_each_deliver_an_etx() {
    let script = "stty raw -echo; printf 'READY:%s:%s\\n' \"$$\" \"$(tty)\"; kill -STOP \"$$\"; IFS= read -r line; etx=$(printf '\\003'); count=0; while [ \"${line#*\"$etx\"}\" != \"$line\" ]; do line=${line#*\"$etx\"}; count=$((count + 1)); done; printf 'ETX:%s\\n' \"$count\"; printf 'DONE\\n'";
    let (mut bridge, mut input, mut control) = launch(script, None);
    let (receiver, reader) = output_events(&mut bridge);
    let mut seen = Vec::new();
    until(&receiver, &mut seen, b"\n");
    let ready = String::from_utf8_lossy(&seen);
    let mut fields = ready.trim().split(':');
    assert_eq!(fields.next(), Some("READY"), "{ready}");
    let pid = fields.next().unwrap().to_string();
    let tty = fields.next().unwrap().to_string();
    input.write_all(&input_frame(0, &[b'x'; 4096])).unwrap();
    control
        .write_all(&crate::pty::frame(2, &[100, 0, 40, 0]))
        .unwrap();
    control
        .write_all(&crate::pty::frame(2, &[101, 0, 40, 0]))
        .unwrap();
    let wait_size = |expected: &[u8]| {
        let started = Instant::now();
        loop {
            let size = Command::new("stty")
                .args(["-F", &tty, "size"])
                .output()
                .unwrap();
            assert!(size.status.success());
            if size.stdout == expected {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "resize was not applied"
            );
            thread::sleep(Duration::from_millis(10));
        }
    };
    wait_size(b"40 101\n");
    for generation in 1..=20 {
        control.write_all(&interrupt_frame(generation)).unwrap();
    }
    input.write_all(&input_frame(20, b"\n")).unwrap();
    assert!(
        Command::new("/bin/kill")
            .args(["-CONT", &pid])
            .status()
            .unwrap()
            .success()
    );
    while !seen.windows(4).any(|bytes| bytes == b"DONE") {
        seen.extend(
            receiver
                .recv_timeout(Duration::from_secs(5))
                .unwrap_or_else(|error| panic!("raw read timed out ({error}): {seen:?}")),
        );
    }
    assert!(
        String::from_utf8_lossy(&seen).contains("ETX:20\n"),
        "{seen:?}"
    );
    assert_eq!(finished(&mut bridge.child).code(), Some(0));
    reader.join().unwrap();
}

#[test]
fn interrupt_with_disabled_vintr_delivers_literal_etx() {
    let script = "stty -echo intr undef; printf 'READY\\n'; od -An -tu1 -N 1; printf 'DONE\\n'";
    let (mut bridge, mut input, mut control) = launch(script, None);
    let (receiver, reader) = output_events(&mut bridge);
    let mut seen = Vec::new();
    until(&receiver, &mut seen, b"READY");
    control.write_all(&interrupt_frame(1)).unwrap();
    input.write_all(&input_frame(1, b"\n")).unwrap();
    until(&receiver, &mut seen, b"DONE");
    assert!(
        String::from_utf8_lossy(&seen).contains(" 3\r\n"),
        "{seen:?}"
    );
    assert_eq!(finished(&mut bridge.child).code(), Some(0));
    reader.join().unwrap();
}

#[test]
fn interrupt_uses_remapped_vintr() {
    let script = "stty -echo intr '^]'; trap 'kill \"$sleeper\" 2>/dev/null; wait \"$sleeper\" 2>/dev/null; printf INTERRUPTED\\n' INT; sleep 30 & sleeper=$!; printf 'READY\\n'; wait \"$sleeper\"; IFS= read -r line; printf 'LINE:%s:END\\n' \"$line\"";
    let (mut bridge, mut input, mut control) = launch(script, None);
    let (receiver, reader) = output_events(&mut bridge);
    let mut seen = Vec::new();
    until(&receiver, &mut seen, b"READY");
    input.write_all(&input_frame(0, b"OLD\n")).unwrap();
    control.write_all(&interrupt_frame(1)).unwrap();
    until(&receiver, &mut seen, b"INTERRUPTED");
    input.write_all(&input_frame(1, b"NEW\n")).unwrap();
    until(&receiver, &mut seen, b":END");
    assert!(
        String::from_utf8_lossy(&seen).contains("LINE:NEW:END"),
        "{seen:?}"
    );
    assert_eq!(finished(&mut bridge.child).code(), Some(0));
    reader.join().unwrap();
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
    let packet = input_frame(0, &[b'x'; 4096]);
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
    control.write_all(&interrupt_frame(1)).unwrap();
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
