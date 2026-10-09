use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(super) struct Mode {
    child: Child,
    control: Option<ChildStdin>,
    ready: Option<JoinHandle<()>>,
    tty: File,
    saved: String,
}

pub(super) fn stty(tty: &File, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("stty")
        .args(arguments)
        .env("LC_ALL", "C")
        .stdin(Stdio::from(tty.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("Could not run stty: {error}"))?;
    if !output.status.success() {
        return Err("stty could not access the terminal".into());
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|_| "Invalid stty output".into())
}

impl Mode {
    pub(super) fn open(tty: File, bash: &Path) -> Result<Self, String> {
        let saved = stty(&tty, &["-g"])?;
        let mut child = Command::new(bash)
            .args(["--noprofile", "--norc", "-c", include_str!("../unix.sh")])
            .env_remove("BASH_ENV")
            .env_remove("ENV")
            .env_remove("OPENROUTER_API_KEY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("Could not start Unix terminal input: {error}"))?;
        let control = child.stdin.take();
        let output = child.stdout.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        let ready = thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(output).read_line(&mut line);
            let _ = sender.send(line);
        });
        let mut mode = Self {
            child,
            control,
            ready: Some(ready),
            tty,
            saved,
        };
        if receiver.recv_timeout(Duration::from_secs(5)).as_deref() != Ok("READY\n") {
            return Err("Unix terminal input did not initialize. Use jecode --plain.".into());
        }
        let _ = mode.ready.take().unwrap().join();
        Ok(mode)
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
        let failed = !matches!(self.child.try_wait(), Ok(Some(status)) if status.success());
        if failed {
            let _ = write!(std::io::stdout(), "{}", crate::tui::terminal::LEAVE);
            let _ = std::io::stdout().flush();
        }
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        if let Some(ready) = self.ready.take() {
            let _ = ready.join();
        }
        // A second restoration covers a killed or failed guardian as well.
        if let Err(error) = stty(&self.tty, &[&self.saved]) {
            eprintln!("Could not restore terminal settings: {error}");
        }
    }
}
