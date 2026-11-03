//! Persistent stdio over the same owned process-tree boundary as commands.

use crate::cancel::Cancellation;
use std::io::Write;
use std::process::{Child, ChildStderr, ChildStdout, Command};
use std::time::{Duration, Instant};

pub struct Duplex {
    child: Child,
    pub stdin: Option<Box<dyn Write + Send>>,
    pub stdout: Option<ChildStdout>,
    pub stderr: Option<ChildStderr>,
    #[cfg(windows)]
    completion: Option<super::windows::Report>,
    #[cfg(windows)]
    control: Option<std::fs::File>,
    #[cfg(windows)]
    target_pid: u32,
    #[cfg(unix)]
    _owner: super::unix::Owner,
}

impl Duplex {
    pub fn spawn_cancellable(command: &mut Command, cancel: &Cancellation) -> Result<Self, String> {
        if cancel.requested() {
            return Err("Operation cancelled".into());
        }
        #[cfg(windows)]
        let mut spawned =
            super::windows::spawn_duplex(command, cancel).map_err(|error| error.message)?;
        #[cfg(unix)]
        let mut spawned =
            super::unix::spawn_duplex(command, cancel).map_err(|error| error.message)?;
        #[cfg(windows)]
        {
            spawned
                .writer
                .join()
                .map_err(|_| "Process input worker failed".to_string())?
                .map_err(|error| format!("Could not initialize process input: {error}"))?;
        }
        Ok(Self {
            stdin: spawned
                .stdin
                .take()
                .map(|writer| Box::new(writer) as Box<dyn Write + Send>),
            stdout: Some(spawned.stdout),
            stderr: spawned.child.stderr.take(),
            child: spawned.child,
            #[cfg(windows)]
            completion: spawned.completion,
            #[cfg(windows)]
            control: spawned.control,
            #[cfg(windows)]
            target_pid: spawned.target_pid,
            #[cfg(unix)]
            _owner: spawned.owner,
        })
    }

    pub fn id(&self) -> u32 {
        #[cfg(windows)]
        {
            self.target_pid
        }
        #[cfg(unix)]
        {
            self.child.id()
        }
    }

    /// Stops all descendants even if a server ignored shutdown or closed stdout.
    pub fn kill_tree(&mut self) -> Result<(), String> {
        self.stdin.take();
        if self
            .child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Ok(());
        }
        #[cfg(windows)]
        {
            let result = super::windows::terminate(
                &mut self.child,
                self.control.as_ref(),
                &mut self.completion,
            )
            .map(|_| ());
            super::windows::reap(&mut self.child);
            result
        }
        #[cfg(unix)]
        {
            super::unix::terminate(&mut self.child)
        }
    }

    pub fn wait_timeout(&mut self, timeout: Duration) -> Result<bool, String> {
        let started = Instant::now();
        loop {
            if self
                .child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Ok(true);
            }
            if started.elapsed() >= timeout {
                return Ok(false);
            }
            std::thread::sleep(
                Duration::from_millis(10).min(timeout.saturating_sub(started.elapsed())),
            );
        }
    }
}

impl Drop for Duplex {
    fn drop(&mut self) {
        let _ = self.kill_tree();
    }
}

#[cfg(test)]
mod tests;
