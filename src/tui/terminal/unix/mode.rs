use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(super) struct Mode {
    child: Child,
    control: Option<ChildStdin>,
    reports: Option<JoinHandle<()>>,
    tty: File,
    saved: String,
    /// The guardian reported READY, so it pushed and set the window title.
    entered: bool,
    title_enabled: bool,
}

pub(super) fn stty(tty: &File, arguments: &[&str]) -> Result<String, String> {
    let mut command = Command::new(crate::native::helper("terminal_unix")?);
    command
        .arg("mode")
        .args(arguments)
        .env("LC_ALL", "C")
        .stdin(Stdio::from(tty.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::null());
    let output = crate::auth::isolate_environment(&mut command)
        .output()
        .map_err(|error| format!("Could not query native terminal settings: {error}"))?;
    if !output.status.success() {
        return Err("Could not access native terminal settings".into());
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|_| "Invalid native terminal settings".into())
}

/// `rows columns` as (width, height), when both are positive.
fn parse_size(text: &str) -> Option<(usize, usize)> {
    let mut values = text.split_whitespace().map(str::parse::<usize>);
    match (values.next(), values.next(), values.next()) {
        (Some(Ok(height)), Some(Ok(width)), None) if height > 0 && width > 0 => {
            Some((width, height))
        }
        _ => None,
    }
}

impl Mode {
    /// Starts the guardian. It returns the terminal's size and gives every later
    /// one to `report`, which returns false once nobody is listening.
    pub(super) fn open(
        tty: File,
        mut report: impl FnMut((usize, usize)) -> bool + Send + 'static,
    ) -> Result<(Self, (usize, usize)), String> {
        let saved = stty(&tty, &["-g"])?;
        let title_enabled = std::env::var_os("WT_SESSION").is_none();
        let mut command = Command::new(crate::native::helper("terminal_unix")?);
        command
            .arg("guard")
            .env_remove("BASH_ENV")
            .env_remove("ENV")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = crate::auth::isolate_environment(&mut command)
            .spawn()
            .map_err(|error| format!("Could not start Unix terminal input: {error}"))?;
        let control = child.stdin.take();
        let output = child.stdout.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        let reports = thread::spawn(move || {
            let mut output = BufReader::new(output);
            let mut line = String::new();
            let _ = output.read_line(&mut line);
            let _ = sender.send(std::mem::take(&mut line));
            // Ends with the guardian, which closes this pipe when it exits.
            while matches!(output.read_line(&mut line), Ok(count) if count > 0) {
                if parse_size(&line).is_some_and(|size| !report(size)) {
                    break;
                }
                line.clear();
            }
        });
        let mut mode = Self {
            child,
            control,
            reports: Some(reports),
            tty,
            saved,
            entered: false,
            title_enabled,
        };
        let line = receiver.recv_timeout(Duration::from_secs(5)).ok();
        let Some(measured) = line.as_deref().and_then(|line| line.strip_prefix("READY")) else {
            return Err("Unix terminal input did not initialize. Use jecode --plain.".into());
        };
        mode.entered = true;
        let size =
            parse_size(measured).ok_or("Could not measure terminal size. Use jecode --plain.")?;
        Ok((mode, size))
    }

    pub(super) fn check(&mut self) -> Result<(), String> {
        match self.child.try_wait() {
            Ok(None) => Ok(()),
            _ => Err("Unix terminal input closed. Use jecode --plain.".into()),
        }
    }

    pub(super) fn stop_reads(&self) {
        // A failed guardian may already have restored canonical (blocking) input.
        let _ = stty(&self.tty, &["-icanon", "min", "0", "time", "1"]);
    }

    #[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
    pub(super) fn close_for_fixture(&mut self) {
        self.child.kill().unwrap();
        let _ = self.child.wait().unwrap();
    }

    #[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
    pub(super) fn stop_for_fixture(&mut self) {
        assert!(
            Command::new("kill")
                .args(["-STOP", &self.child.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
    }
}

impl Drop for Mode {
    fn drop(&mut self) {
        // Closing, rather than writing, also works after a failed initialization.
        drop(self.control.take());
        let started = Instant::now();
        while matches!(self.child.try_wait(), Ok(None))
            && started.elapsed() < Duration::from_secs(2)
        {
            thread::sleep(Duration::from_millis(10));
        }
        // Reap a stuck guardian before writing fallback escapes. Otherwise it
        // could resume after our pop and pop an enclosing application's title.
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
        let status = self.child.wait();
        let failed = !matches!(status, Ok(status) if status.success());
        // Exit code 4 means the guardian could not pop its saved title. A
        // signal means it could not run cleanup; READY confirms it pushed.
        let title = self.title_enabled
            && (matches!(status, Ok(status) if status.code() == Some(4))
                || self.entered && matches!(status, Ok(status) if status.code().is_none()));
        if failed {
            let restore = if title {
                crate::tui::terminal::RESTORE_TITLE
            } else {
                ""
            };
            let _ = write!(
                std::io::stdout(),
                "{}{restore}",
                crate::tui::terminal::LEAVE
            );
            let _ = std::io::stdout().flush();
        }
        if let Some(reports) = self.reports.take() {
            let _ = reports.join();
        }
        // A second restoration covers a killed or failed guardian as well.
        if let Err(error) = stty(&self.tty, &[&self.saved]) {
            eprintln!("Could not restore terminal settings: {error}");
        }
    }
}
