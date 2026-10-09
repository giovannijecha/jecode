use crate::test_support::Directory;
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(super) struct Pty {
    directory: Directory,
    child: Child,
    input: Option<ChildStdin>,
    output: Receiver<Vec<u8>>,
    reader: Option<JoinHandle<()>>,
    bytes: Vec<u8>,
    owner: Option<u32>,
    tty: Option<PathBuf>,
}

impl Pty {
    pub(super) fn new(case: &str) -> Self {
        let directory = Directory::new();
        let startup = directory.path().join("unexpected-startup.sh");
        fs::write(&startup, "printf 'UNEXPECTED_BASH_ENV\\n'; exit 7\n").unwrap();
        let executable = quote(std::env::current_exe().unwrap().to_str().unwrap());
        let fixture = "tui::terminal::unix::native_tests::native_pty_fixture";
        let run = format!("{executable} --exact {fixture} --ignored --nocapture --test-threads=1");
        let script = if case == "owner-death" {
            format!(
                "stty rows 24 cols 80; saved=$(stty -g); {run}; n=0; while [ \"$(stty -g)\" != \"$saved\" ] && [ $n -lt 30 ]; do sleep 0.1; n=$((n + 1)); done; if [ \"$(stty -g)\" = \"$saved\" ]; then printf '\\nPTY_OWNER_RESTORED\\n'; else exit 1; fi"
            )
        } else {
            format!("stty rows 24 cols 80; exec {run}")
        };
        let mut helper = Command::new("script");
        #[cfg(target_os = "linux")]
        helper.args(["-qefc", &script, "/dev/null"]);
        #[cfg(target_os = "macos")]
        helper.args(["-q", "/dev/null", "/bin/sh", "-c", &script]);
        let mut child = helper
            .env("SHELL", "/bin/sh")
            .env("TERM", "xterm-256color")
            .env("BASH_ENV", startup)
            .env("ENV", directory.path().join("unused-env.sh"))
            .env("JECODE_TUI_FIXTURE", directory.path())
            .env("JECODE_TUI_CASE", case)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("native PTY checks require the system script tool");
        let input = child.stdin.take();
        let mut pipe = child.stdout.take().unwrap();
        let (sender, output) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        if sender.send(buffer[..count].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            directory,
            child,
            input,
            output,
            reader: Some(reader),
            bytes: vec![],
            owner: None,
            tty: None,
        }
    }

    pub(super) fn directory(&self) -> &Path {
        self.directory.path()
    }

    pub(super) fn send(&mut self, bytes: &[u8]) {
        self.input.as_mut().unwrap().write_all(bytes).unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
    }

    fn poll(&mut self) {
        if let Ok(bytes) = self.output.recv_timeout(Duration::from_millis(20)) {
            self.bytes.extend(bytes);
            assert!(
                self.bytes.len() <= 4 * 1024 * 1024,
                "PTY output exceeded fixture limit"
            );
            if self.owner.is_none() {
                let text = String::from_utf8_lossy(&self.bytes);
                if let Some(line) = text.lines().find(|line| line.starts_with("PTY_META|")) {
                    let fields: Vec<_> = line.trim().split('|').collect();
                    if fields.len() == 3 {
                        self.owner = fields[1].parse().ok();
                        self.tty = Some(PathBuf::from(fields[2]));
                    }
                }
            }
        }
    }

    pub(super) fn wait(&mut self, text: &str) -> usize {
        self.wait_after(text, 0)
    }

    pub(super) fn wait_after(&mut self, text: &str, start: usize) -> usize {
        let at = Instant::now();
        loop {
            let visible = plain(&self.bytes);
            if let Some(offset) = visible[start..]
                .windows(text.len())
                .position(|bytes| bytes == text.as_bytes())
            {
                return start + offset + text.len();
            }
            assert!(
                at.elapsed() < Duration::from_secs(10),
                "PTY did not show {text:?}: {}",
                String::from_utf8_lossy(&self.bytes)
            );
            self.poll();
        }
    }

    pub(super) fn wait_file(&mut self, name: &str) {
        let at = Instant::now();
        while !self.directory.path().join(name).exists() {
            assert!(
                at.elapsed() < Duration::from_secs(10),
                "Native command did not start: {}",
                String::from_utf8_lossy(&self.bytes)
            );
            self.poll();
        }
    }

    pub(super) fn resize(&mut self, columns: usize, rows: usize) {
        let tty = File::open(self.tty.as_ref().unwrap()).unwrap();
        assert!(
            Command::new("stty")
                .args(["rows", &rows.to_string(), "cols", &columns.to_string()])
                .stdin(Stdio::from(tty))
                .status()
                .unwrap()
                .success()
        );
        let at = Instant::now();
        while at.elapsed() < Duration::from_millis(350) {
            self.poll();
        }
    }

    pub(super) fn kill_owner(&mut self) {
        assert!(
            Command::new("kill")
                .args(["-KILL", &self.owner.unwrap().to_string()])
                .status()
                .unwrap()
                .success()
        );
        self.owner = None;
    }

    pub(super) fn finish(&mut self, marker: &str) {
        self.wait(marker);
        drop(self.input.take());
        let at = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "Native PTY exited {status}");
                break;
            }
            assert!(
                at.elapsed() < Duration::from_secs(3),
                "Native PTY did not exit"
            );
            self.poll();
        }
        self.owner = None;
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
        while let Ok(bytes) = self.output.try_recv() {
            self.bytes.extend(bytes);
        }
        assert!(!String::from_utf8_lossy(&self.bytes).contains("UNEXPECTED_BASH_ENV"));
        let entered = self
            .bytes
            .windows(8)
            .filter(|bytes| *bytes == b"\x1b[?1049h")
            .count();
        let left = self
            .bytes
            .windows(8)
            .filter(|bytes| *bytes == b"\x1b[?1049l")
            .count();
        assert!(
            entered > 0 && entered == left,
            "alternate screen was not restored ({entered} entries, {left} exits)"
        );
        assert!(!self.bytes.windows(4).any(|bytes| bytes == b"\x1b[3J"));
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.take() {
            let _ = Command::new("kill")
                .args(["-KILL", &owner.to_string()])
                .status();
        }
        drop(self.input.take());
        let at = Instant::now();
        while matches!(self.child.try_wait(), Ok(None)) && at.elapsed() < Duration::from_secs(1) {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn plain(bytes: &[u8]) -> Vec<u8> {
    let mut visible = vec![];
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"\x1b[") {
            index += 2;
            while index < bytes.len() && !(0x40..=0x7e).contains(&bytes[index]) {
                index += 1;
            }
            index += 1;
        } else {
            visible.push(bytes[index]);
            index += 1;
        }
    }
    visible
}
