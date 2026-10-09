use super::RunError;
use crate::cancel::Cancellation;
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

mod input;

pub(super) struct Spawned {
    pub child: Child,
    pub stdout: ChildStdout,
    pub writer: Option<JoinHandle<io::Result<()>>>,
    pub owner: Owner,
}

pub(super) struct Owner {
    control: Option<ChildStdin>,
    stop: Arc<AtomicBool>,
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Only the owner holds this write end. No target inherits it.
        drop(self.control.take());
    }
}

pub(super) fn spawn(
    command: &Command,
    bytes: Option<Vec<u8>>,
    cancellation: &Cancellation,
) -> Result<Spawned, RunError> {
    let pipe = bytes.as_ref().map(|_| input::Pipe::open()).transpose()?;
    let bash = crate::tools::find_bash().map_err(before_start)?;
    let mut helper = Command::new(bash);
    helper
        .args([
            "--noprofile",
            "--norc",
            "-c",
            include_str!("unix.sh"),
            "jecode-process",
        ])
        .arg(
            pipe.as_ref()
                .map_or(std::path::Path::new("/dev/null"), |pipe| pipe.path()),
        )
        .arg(command.get_program())
        .args(command.get_args())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(directory) = command.get_current_dir() {
        helper.current_dir(directory);
    }
    for (key, value) in command.get_envs() {
        if let Some(value) = value {
            helper.env(key, value);
        } else {
            helper.env_remove(key);
        }
    }
    helper
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env_remove("SHELLOPTS")
        .env_remove("BASHOPTS");
    let mut child = helper
        .spawn()
        .map_err(|error| before_start(format!("Could not start Unix process guardian: {error}")))?;
    let owner = Owner {
        control: child.stdin.take(),
        stop: Arc::new(AtomicBool::new(false)),
    };
    let mut stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    let handshake = thread::spawn(move || {
        let mut header = [0; 6];
        let result = stdout.read_exact(&mut header).map(|_| header);
        let _ = sender.send((stdout, result));
    });
    let started = Instant::now();
    loop {
        match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok((stdout, Ok(header))) if header == *b"READY\n" => {
                let _ = handshake.join();
                let writer = pipe.zip(bytes).map(|(pipe, bytes)| {
                    pipe.send(bytes, Arc::clone(&owner.stop), cancellation.clone())
                });
                return Ok(Spawned {
                    child,
                    stdout,
                    writer,
                    owner,
                });
            }
            Ok(_) => {
                let _ = super::terminate(&mut child);
                let _ = handshake.join();
                return Err(before_start("Could not start process: executable was not found or guardian initialization failed".into()));
            }
            Err(mpsc::RecvTimeoutError::Timeout)
                if started.elapsed() < Duration::from_secs(5) && !cancellation.requested() => {}
            Err(_) => {
                let _ = super::terminate(&mut child);
                let _ = handshake.join();
                return Err(before_start(if cancellation.requested() {
                    "Operation cancelled".into()
                } else {
                    "Unix process guardian did not initialize".into()
                }));
            }
        }
    }
}

fn before_start(message: String) -> RunError {
    RunError {
        started: false,
        message,
    }
}

#[cfg(test)]
mod tests;
