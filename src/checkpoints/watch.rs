//! Windows change notifications through an owned .NET helper process. The
//! helper only reports paths; the workspace rereads and records them.
use super::workspace::{Health, Signals, Watching, Workspace};
use crate::cancel::Cancellation;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SOURCE: &str = include_str!("watcher.cs");
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const READY_LIMIT: Duration = Duration::from_secs(20);
const RESTART_LIMIT: u32 = 5;
const DIRTY_LIMIT: usize = 4096;
static HELPER: Mutex<Option<Option<PathBuf>>> = Mutex::new(None);

pub struct Watcher {
    child: Child,
    input: ChildStdin,
    next: u64,
}

impl Watcher {
    fn start(root: &Path, signals: Arc<Signals>, generation: u64) -> Result<Self, String> {
        let executable = {
            let mut helper = HELPER.lock().unwrap_or_else(|e| e.into_inner());
            helper
                .get_or_insert_with(|| {
                    crate::process::compile_helper("watcher-v1", SOURCE, &Cancellation::default())
                })
                .clone()
                .ok_or("the notification helper could not be compiled")?
        };
        // .NET rejects verbatim `\\?\` roots.
        let mut child = Command::new(executable)
            .arg(super::identity::plain(root))
            .arg(std::process::id().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .env_remove("OPENROUTER_API_KEY")
            .spawn()
            .map_err(|error| format!("the notification helper did not start: {error}"))?;
        let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("the notification helper has no pipes".into());
        };
        std::thread::Builder::new()
            .name("jecode-watcher".into())
            .spawn(move || read(output, &signals, generation))
            .map_err(|error| format!("could not read notifications: {error}"))?;
        Ok(Self {
            child,
            input,
            next: 0,
        })
    }

    /// Checks whether the helper responds on its input channel. The echo is
    /// not ordered with filesystem notification callbacks.
    pub fn marker(&mut self) -> Option<u64> {
        self.next += 1;
        writeln!(self.input, "S {}", self.next).ok()?;
        self.input.flush().ok()?;
        Some(self.next)
    }

    /// Ends the helper as a crash would.
    #[cfg(test)]
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    #[cfg(test)]
    pub fn markers_sent(&self) -> u64 {
        self.next
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read(output: std::process::ChildStdout, signals: &Signals, generation: u64) {
    let current = |signals: &Signals| signals.lock().watcher == generation;
    for line in BufReader::new(output).split(b'\n') {
        let Ok(line) = line else {
            break;
        };
        let line = String::from_utf8_lossy(&line);
        let line = line.trim_end_matches('\r');
        if !current(signals) {
            return;
        }
        let (kind, rest) = line.split_at(line.len().min(1));
        let rest = rest.strip_prefix(' ').unwrap_or(rest);
        signals.update(|health| match kind {
            "R" => health.watching = Watching::Active,
            "C" | "D" => queue_path(health, rest),
            "N" => {
                for path in rest.split('\t') {
                    queue_path(health, path);
                }
            }
            "O" => {
                health.dirty.clear();
                health.rescan = Some("change notifications overflowed".into());
                health.event = Some(Instant::now());
            }
            "S" => health.marker = health.marker.max(rest.parse().unwrap_or(0)),
            "E" => health.watching = Watching::Lost(rest.to_string()),
            _ => {}
        });
    }
    signals.update(|health| {
        if health.watcher == generation
            && !health.stopped
            && !matches!(health.watching, Watching::Lost(_))
        {
            health.watching = Watching::Lost("the notification helper stopped".into());
        }
    });
}

/// Excluded changes do not delay command completion or consume queue space.
/// A full scan supersedes every queued path; later names cannot grow the queue.
pub(super) fn queue_path(health: &mut Health, path: &str) {
    if health.rescan.is_some() {
        return;
    }
    let path = path.replace('\\', "/");
    if !super::scan::in_scope(&health.policy, &health.excluded, &path, false)
        && !super::scan::in_scope(&health.policy, &health.excluded, &path, true)
    {
        return;
    }
    health.event = Some(Instant::now());
    if health.dirty.contains(&path) {
        return;
    }
    if health.dirty.len() >= DIRTY_LIMIT {
        health.dirty.clear();
        health.rescan = Some("change notification queue exceeded 4096 paths".into());
        return;
    }
    health.dirty.insert(path);
}

impl Workspace {
    /// Starts notifications and waits until the helper watches the root.
    pub(super) fn start_watcher(&self) {
        let generation = {
            let mut health = self.signals.lock();
            health.watcher += 1;
            health.watching = Watching::Starting;
            health.watcher
        };
        let mut slot = self.watcher.lock().unwrap_or_else(|e| e.into_inner());
        *slot = None;
        let watcher = match Watcher::start(&self.root, self.signals.clone(), generation) {
            Ok(watcher) => watcher,
            Err(error) => {
                self.signals
                    .update(|health| health.watching = Watching::Unavailable(error));
                return;
            }
        };
        *slot = Some(watcher);
        drop(slot);
        let started = Instant::now();
        let mut health = self.signals.lock();
        while health.watching == Watching::Starting && started.elapsed() < READY_LIMIT {
            health = self
                .signals
                .changed
                .wait_timeout(health, Duration::from_millis(100))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        if health.watching == Watching::Starting {
            health.watching = Watching::Lost("the notification helper did not become ready".into());
        }
    }

    /// Replaces a lost helper; after repeated losses, falls back to scans.
    pub(super) fn restart_watcher(&self) {
        let reason = match &self.signals.lock().watching {
            Watching::Lost(reason) => reason.clone(),
            _ => return,
        };
        let attempt = {
            let mut health = self.signals.lock();
            health.restarts += 1;
            health.restarts
        };
        if attempt > RESTART_LIMIT {
            *self.watcher.lock().unwrap_or_else(|e| e.into_inner()) = None;
            self.signals.update(|health| {
                health.watching =
                    Watching::Unavailable(format!("notifications stopped repeatedly: {reason}"));
                health.rescan = Some("change notifications stopped".into());
            });
            return;
        }
        self.start_watcher();
        // Changes between the loss and the restart were not reported.
        self.signals.update(|health| {
            health.rescan = Some(format!("change notifications restarted: {reason}"))
        });
    }
}
