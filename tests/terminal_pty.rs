//! Exercise the real executable through an isolated Linux PTY, with no shell.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#![allow(unsafe_code)]
use std::{
    ffi::{CStr, c_char, c_ulong},
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::fd::{AsRawFd, FromRawFd},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
struct Attributes {
    input: u32,
    output: u32,
    control: u32,
    local: u32,
    line: u8,
    characters: [u8; 32],
    input_speed: u32,
    output_speed: u32,
}
unsafe extern "C" {
    fn posix_openpt(flags: i32) -> i32;
    fn grantpt(fd: i32) -> i32;
    fn unlockpt(fd: i32) -> i32;
    fn ptsname_r(fd: i32, buffer: *mut c_char, size: usize) -> i32;
    fn ioctl(fd: i32, request: c_ulong, ...) -> i32;
    fn tcgetattr(fd: i32, result: *mut Attributes) -> i32;
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn attributes(slave: &File) -> Attributes {
    let mut value = Attributes::default();
    // SAFETY: Linux x86_64 termios layout, owned live PTY descriptor.
    assert_eq!(unsafe { tcgetattr(slave.as_raw_fd(), &mut value) }, 0);
    value
}
fn resize(master: &File, rows: u16, columns: u16) {
    let size = [rows, columns, 0, 0];
    // SAFETY: TIOCSWINSZ reads four u16 values from a live PTY.
    assert_eq!(unsafe { ioctl(master.as_raw_fd(), 0x5414, &size) }, 0);
}
fn until(master: &mut File, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut output = Vec::new();
    while Instant::now() < deadline {
        let mut bytes = [0; 8192];
        match master.read(&mut bytes) {
            Ok(count) => output.extend_from_slice(&bytes[..count]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => panic!(
                "PTY read: {error}; output: {}",
                String::from_utf8_lossy(&output)
            ),
        }
        assert!(output.len() <= 1_048_576);
        let text = String::from_utf8_lossy(&output);
        if text.contains(needle) {
            return text.into_owned();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "missing {needle:?}; output: {}",
        String::from_utf8_lossy(&output)
    );
}

#[test]
fn real_terminal_stream_resize_paste_cancel_and_restore() {
    // SAFETY: allocates a fresh PTY, O_RDWR | O_NOCTTY | O_NONBLOCK | O_CLOEXEC.
    let fd = unsafe { posix_openpt(2 | 0x100 | 0x800 | 0x80000) };
    assert!(fd >= 0, "PTY required: {}", io::Error::last_os_error());
    // SAFETY: transfer the one fresh descriptor into File ownership.
    let mut master = unsafe { File::from_raw_fd(fd) };
    let mut name = [0 as c_char; 256];
    // SAFETY: live PTY descriptor and a correctly sized writable name buffer.
    unsafe {
        assert_eq!(grantpt(fd), 0);
        assert_eq!(unlockpt(fd), 0);
        assert_eq!(ptsname_r(fd, name.as_mut_ptr(), name.len()), 0);
    }
    // SAFETY: successful ptsname_r terminates the name in the provided buffer.
    let path = unsafe { CStr::from_ptr(name.as_ptr()) }.to_str().unwrap();
    let slave = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    resize(&master, 24, 80);
    let before = attributes(&slave);
    let mut child = Process(
        Command::new(env!("CARGO_BIN_EXE_jecode"))
            .arg("--demo")
            .env("NO_COLOR", "1")
            .env("JECODE_REDUCED_MOTION", "1")
            .env("TERM", "xterm-256color")
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave.try_clone().unwrap()))
            .spawn()
            .unwrap(),
    );
    let first = until(&mut master, "Local demo");
    assert!(first.contains("All activity is simulated"));
    assert!(
        !first.contains("jecode\r\n"),
        "no interactive brand heading"
    );
    assert!(!first.contains("1049"));
    assert!(!first.contains("\x1b[36m"));
    assert_ne!(attributes(&slave), before);
    master
        .write_all(b"\x1b[200~/code\n\x03\x11\x1b[201~")
        .unwrap();
    let paste = until(&mut master, "??|");
    assert!(paste.contains("/code"));
    assert!(!paste.contains("Streaming locally"));
    // Ctrl+C clears the inert draft; no process termination in raw mode.
    master.write_all(b"\x03/code\r").unwrap();
    let completed = until(&mut master, "No compiler or command was invoked.");
    assert!(completed.contains("Unicode sample:"));
    assert!(!completed.contains("```"));
    master.write_all(b"/tools-error\r").unwrap();
    let active = until(&mut master, "Exploring workspace");
    assert!(
        active.contains("⠿ Exploring workspace"),
        "static indicator: {active}"
    );
    let completed = until(&mut master, "completed activity stays");
    assert!(completed.contains("Exploration finished with errors"));
    assert!(completed.contains("permission denied"));
    assert!(
        !completed.contains("\x1b[0;"),
        "NO_COLOR must include tool rows"
    );
    master.write_all(b"/edit\r").unwrap();
    let approval = until(&mut master, "Apply this change?");
    assert!(approval.contains("-       2"));
    assert!(approval.contains("+       3"));
    assert!(approval.contains("› Deny"));
    master.write_all(b"\x1b[C\r").unwrap();
    let edited = until(&mut master, "Simulated edit complete");
    assert!(edited.contains("Approved once"));
    assert!(!edited.contains("\x1b[0;"), "diff must respect NO_COLOR");
    master.write_all(b"/command-error\r").unwrap();
    until(&mut master, "Run this command?");
    master.write_all(b"\x1b[C\r").unwrap();
    let output = until(&mut master, "running 2 tests");
    assert!(!output.contains("Simulated command complete"));
    let completed = until(&mut master, "Simulated command complete");
    assert!(completed.contains("exit 101"));
    assert!(completed.contains("expected: 3, received: 2"));
    master.write_all(b"/command\r").unwrap();
    until(&mut master, "Run this command?");
    master.write_all(b"\r").unwrap();
    let denied = until(&mut master, "Denied");
    assert!(!denied.contains("running 2 tests"));
    master.write_all(b"/long\r").unwrap();
    until(&mut master, "⠿ Streaming");
    master.write_all(b"draft\x1b").unwrap();
    until(&mut master, "Interrupted / partial response kept");
    for (rows, columns) in [(12, 40), (24, 80), (16, 60), (24, 80)] {
        resize(&master, rows, columns);
        let resized = until(&mut master, "Local demo");
        assert!(resized.contains("\r\x1b[J"));
        assert!(!resized.contains("\x1b[2J"));
        assert!(!resized.contains("\x1b[3J"));
        assert!(!resized.contains("useful harness"));
        assert!(!resized.contains("fn main"));
        assert!(resized.contains("draft"));
    }
    master.write_all(&[17]).unwrap();
    let exit = until(&mut master, "\x1b[?25h");
    assert!(!exit.contains("1049"));
    assert!(exit.contains("\x1b[?25h"));
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "preview did not exit");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(attributes(&slave), before);
}
