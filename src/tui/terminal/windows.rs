use super::{Geometry, Input};
use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Receiver},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
mod recovery;

static NEXT_CHANNEL: AtomicU64 = AtomicU64::new(0);

pub struct Terminal {
    pub input: Receiver<Input>,
    pub size: (usize, usize),
    control: ChildStdin,
    child: Child,
    reader: Option<JoinHandle<()>>,
    modes: Option<(u32, u32, u32)>,
}

impl Terminal {
    pub fn open(_bash: &std::path::Path) -> Result<Self, String> {
        let name = format!(
            "jecode-terminal-{}-{}",
            std::process::id(),
            NEXT_CHANNEL.fetch_add(1, Ordering::Relaxed)
        );
        let script = include_str!("../terminal.ps1")
            .replace("__JECODE_NATIVE_SOURCE__", include_str!("../terminal.cs"))
            .replace("__JECODE_CHANNEL__", &name);
        let mut child = Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &script,
            ])
            .env_remove("OPENROUTER_API_KEY")
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                format!("Could not start Windows terminal input: {error}. Use jecode --plain.")
            })?;
        let started = Instant::now();
        let channel = loop {
            match OpenOptions::new()
                .read(true)
                .open(format!(r"\\.\pipe\{name}"))
            {
                Ok(file) => break file,
                Err(_)
                    if started.elapsed() < Duration::from_secs(5)
                        && matches!(child.try_wait(), Ok(None)) =>
                {
                    thread::sleep(Duration::from_millis(20))
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "Could not connect Windows terminal input: {error}. Use jecode --plain."
                    ));
                }
            }
        };
        let reader_file = channel;
        let control = child.stdin.take().unwrap();
        let (sender, input) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(reader_file).lines() {
                let event = match line {
                    Ok(line) => parse(&line),
                    Err(error) => Input::Error(format!("Could not read terminal input: {error}")),
                };
                if sender.send(event).is_err() {
                    break;
                }
            }
        });
        let mut terminal = Self {
            input,
            size: (80, 24),
            control,
            child,
            reader: Some(reader),
            modes: None,
        };
        // The adapter enables VT output before its first size event. Do not draw sooner.
        match terminal.input.recv_timeout(Duration::from_secs(5)) {
            Ok(Input::Modes(input, output, code_page)) => {
                terminal.modes = Some((input, output, code_page))
            }
            Ok(Input::Error(error)) => return Err(format!("{error}. Use jecode --plain.")),
            _ => {
                return Err(
                    "Windows terminal modes did not initialize. Use jecode --plain.".into(),
                );
            }
        }
        match terminal.input.recv_timeout(Duration::from_secs(5)) {
            Ok(Input::Size(size)) => {
                terminal.size = (size.width, size.height);
            }
            Ok(Input::Error(error)) => return Err(format!("{error}. Use jecode --plain.")),
            _ => {
                return Err(
                    "Windows terminal input did not initialize. Use jecode --plain.".into(),
                );
            }
        }
        Ok(terminal)
    }

    pub fn take_initial(&mut self) -> Vec<Input> {
        vec![]
    }

    pub fn check(&mut self) -> Result<(), String> {
        match self.child.try_wait() {
            Ok(None) => Ok(()),
            _ => Err("Windows terminal input closed. Use jecode --plain.".into()),
        }
    }

    #[cfg(test)]
    fn close_for_fixture(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.control.write_all(b"S");
        let started = Instant::now();
        while matches!(self.child.try_wait(), Ok(None))
            && started.elapsed() < Duration::from_secs(2)
        {
            thread::sleep(Duration::from_millis(10));
        }
        let failed = !matches!(self.child.try_wait(), Ok(Some(status)) if status.success());
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = write!(io::stdout(), "{}", super::LEAVE);
            let _ = io::stdout().flush();
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        if failed {
            self.restore_modes();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn parse(line: &str) -> Input {
    let fields: Vec<_> = line.split('|').collect();
    match fields.as_slice() {
        ["I", "overflow"] => Input::PasteOverflow,
        ["R", input, output, code_page] => match (input.parse(), output.parse(), code_page.parse())
        {
            (Ok(input), Ok(output), Ok(code_page)) => Input::Modes(input, output, code_page),
            _ => Input::Error("Invalid console modes".into()),
        },
        ["W", amount] => match amount.parse() {
            Ok(amount) => Input::Scroll(amount),
            Err(_) => Input::Error("Invalid console wheel event".into()),
        },
        ["P", text] => {
            let units: Result<Vec<u16>, _> = text.split(',').map(str::parse).collect();
            match units {
                Ok(units) => Input::Paste(units),
                Err(_) => Input::Error("Invalid console paste".into()),
            }
        }
        ["S", width, height, row, column] => {
            match (width.parse(), height.parse(), row.parse(), column.parse()) {
                (Ok(width), Ok(height), Ok(row), Ok(column)) if width > 0 && height > 0 => {
                    Input::Size(Geometry {
                        width,
                        height,
                        row,
                        column,
                    })
                }
                _ => Input::Error("Invalid console size".into()),
            }
        }
        ["K", code, modifiers, character] => {
            match (code.parse(), modifiers.parse(), character.parse()) {
                (Ok(code), Ok(modifiers), Ok(character)) => Input::Key(super::Key {
                    code,
                    modifiers,
                    character,
                }),
                _ => Input::Error("Invalid console key".into()),
            }
        }
        _ => Input::Error(
            line.strip_prefix("E|")
                .unwrap_or("Invalid terminal event")
                .into(),
        ),
    }
}

#[cfg(test)]
mod native_tests;
#[cfg(test)]
#[path = "windows_tests.rs"]
mod tests;
