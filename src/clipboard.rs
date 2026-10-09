//! Clipboard delivery. Native commands receive only UTF-8 stdin; terminal
//! escape sequences are returned for the UI to serialize with its own output.

use crate::cancel::Cancellation;
use std::thread::{self, JoinHandle};

#[cfg(any(windows, target_os = "macos"))]
use crate::process;
#[cfg(any(windows, target_os = "macos"))]
use std::process::Command;
#[cfg(any(windows, target_os = "macos"))]
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub enum Delivery {
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    Confirmed,
    /// A complete OSC 52 packet. Emitting it only confirms a request was sent
    /// to the terminal; the terminal may ignore or reject it.
    #[cfg_attr(windows, allow(dead_code))]
    Terminal(String),
}

pub struct Job {
    cancellation: Cancellation,
    worker: Option<JoinHandle<Result<Delivery, String>>>,
    #[cfg(test)]
    fixture: Option<Result<Delivery, String>>,
}

impl Job {
    pub fn start(text: String, terminal_available: bool) -> Self {
        let cancellation = Cancellation::default();
        let worker_cancellation = cancellation.clone();
        let worker = thread::spawn(move || copy(&text, terminal_available, &worker_cancellation));
        Self {
            cancellation,
            worker: Some(worker),
            #[cfg(test)]
            fixture: None,
        }
    }

    pub fn finished(&self) -> bool {
        #[cfg(test)]
        if self.fixture.is_some() {
            return true;
        }
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }

    pub fn finish(mut self) -> Result<Delivery, String> {
        #[cfg(test)]
        if let Some(result) = self.fixture.take() {
            return result;
        }
        self.worker
            .take()
            .expect("clipboard job has a worker")
            .join()
            .map_err(|_| "Clipboard worker failed".to_string())?
    }

    /// A completed job for TUI tests. It never reads or writes a clipboard.
    #[cfg(test)]
    pub fn fixture(result: Result<Delivery, String>) -> Self {
        Self {
            cancellation: Cancellation::default(),
            worker: None,
            fixture: Some(result),
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(worker) = self.worker.take() {
            // Native delivery runs through the bounded process guardian. Joining
            // here prevents a discarded request from writing after this returns.
            let _ = worker.join();
        }
    }
}

pub fn copy(
    text: &str,
    terminal_available: bool,
    cancellation: &Cancellation,
) -> Result<Delivery, String> {
    if text.is_empty() {
        return Err("There is no text to copy".into());
    }
    if text.contains('\0') {
        return Err("Text containing a NUL character cannot be copied".into());
    }
    if cancellation.requested() {
        return Err("Operation cancelled".into());
    }

    #[cfg(windows)]
    {
        let _ = terminal_available;
        native_copy(windows_command(), text, cancellation)
    }
    #[cfg(target_os = "macos")]
    {
        // pbcopy classifies EPS and RTF signatures as rich data, even when
        // those bytes are the literal text the user chose to copy.
        if text.starts_with("%!PS") || text.starts_with("{\\rtf") {
            return terminal_packet(text, terminal_available, cancellation);
        }
        native_copy(macos_command(), text, cancellation)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        terminal_packet(text, terminal_available, cancellation)
    }
}

// OSC 52 data travels through terminal output, so keep the request bounded.
#[cfg(any(not(windows), test))]
const MAX_OSC52_INPUT_BYTES: usize = 100_000;
#[cfg(any(not(windows), test))]
const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

#[cfg(any(not(windows), test))]
fn terminal_packet(
    text: &str,
    terminal_available: bool,
    cancellation: &Cancellation,
) -> Result<Delivery, String> {
    if !terminal_available {
        return Err("Clipboard access needs an interactive terminal on this platform".into());
    }
    let data = text.as_bytes();
    if data.len() > MAX_OSC52_INPUT_BYTES {
        return Err(format!(
            "Text is too large for terminal clipboard delivery (limit: {MAX_OSC52_INPUT_BYTES} UTF-8 bytes)"
        ));
    }
    if cancellation.requested() {
        return Err("Operation cancelled".into());
    }
    let mut packet = String::with_capacity(7 + data.len().div_ceil(3) * 4);
    packet.push_str("\x1b]52;c;");
    for chunk in data.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        packet.push(BASE64[(a >> 2) as usize] as char);
        packet.push(BASE64[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        packet.push(if chunk.len() > 1 {
            BASE64[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        packet.push(if chunk.len() > 2 {
            BASE64[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    packet.push('\x07');
    Ok(Delivery::Terminal(packet))
}

#[cfg(any(windows, target_os = "macos"))]
fn native_copy(
    mut command: Command,
    text: &str,
    cancellation: &Cancellation,
) -> Result<Delivery, String> {
    let output = process::run_observed(
        &mut command,
        Some(text.as_bytes().to_vec()),
        Duration::from_secs(8),
        1024,
        cancellation,
        None,
        &mut |_| Ok(()),
    )?;
    if output.cancelled {
        return Err("Operation cancelled".into());
    }
    if output.timed_out {
        return Err("Clipboard command timed out".into());
    }
    if output.exit_code != Some(0) {
        // Do not expose process stderr: native clipboard failures can include
        // arbitrary input or environment details.
        return Err("Clipboard command failed".into());
    }
    Ok(Delivery::Confirmed)
}

#[cfg(windows)]
const WINDOWS_SCRIPT: &str = "$ErrorActionPreference = 'Stop'; $reader = [System.IO.StreamReader]::new([Console]::OpenStandardInput(), [System.Text.UTF8Encoding]::new($false, $true), $false); $text = $reader.ReadToEnd(); $reader.Dispose(); Set-Clipboard -Value $text -ErrorAction Stop";

#[cfg(windows)]
fn windows_command() -> Command {
    // Windows PowerShell is an OS-provided boundary. The fixed script reads
    // exact UTF-8 bytes from stdin and writes a string with Set-Clipboard.
    // User content never appears in command arguments or process output.
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Sta",
        "-Command",
        WINDOWS_SCRIPT,
    ]);
    command
}

#[cfg(target_os = "macos")]
fn macos_command() -> Command {
    // macOS provides pbcopy. Its locale selects stdin encoding; forcing UTF-8
    // preserves Unicode even when the parent shell has a different locale.
    let mut command = Command::new("/usr/bin/pbcopy");
    command.env("LC_ALL", "en_US.UTF-8");
    command
}

#[cfg(test)]
mod tests;
