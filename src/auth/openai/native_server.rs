//! One reusable Windows cryptography helper per process. Starting the .NET
//! helper costs tens of milliseconds, and startup reads several account files,
//! so operations share one helper until it exits after a short idle period.
use crate::cancel::Cancellation;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
#[cfg(test)]
use std::sync::PoisonError;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

const FAILED: &str = "Native cryptography operation failed";
const TIMEOUT: Duration = Duration::from_secs(15);
const LINE_LIMIT: u64 = 64 * 1024;

static SERVER: Mutex<Option<Server>> = Mutex::new(None);

struct Server {
    child: Child,
    input: Option<ChildStdin>,
    answers: Receiver<String>,
}

/// Sends one request of newline-free lines and returns the helper's answer.
pub(super) fn request(lines: &[String], cancel: &Cancellation) -> Result<String, String> {
    let mut server = lock(cancel)?;
    // A helper that exited while idle is replaced and the request is sent
    // once more. Repeating these operations is safe without a received answer.
    for _ in 0..2 {
        if cancel.requested() {
            return Err("Operation cancelled".into());
        }
        if server.is_none() {
            *server = Some(Server::start()?);
        }
        match server.as_mut().unwrap().exchange(lines, cancel) {
            Ok(Some(answer)) if answer == "!" => return Err(FAILED.into()),
            Ok(Some(answer)) => return Ok(answer),
            Ok(None) => stop(&mut server),
            Err(error) => {
                // An unanswered request must not be read as the next answer.
                stop(&mut server);
                return Err(error);
            }
        }
    }
    Err(FAILED.into())
}

fn lock(cancel: &Cancellation) -> Result<MutexGuard<'static, Option<Server>>, String> {
    loop {
        if cancel.requested() {
            return Err("Operation cancelled".into());
        }
        match SERVER.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(error)) => return Ok(error.into_inner()),
            Err(TryLockError::WouldBlock) => cancel.wait(Duration::from_millis(10))?,
        }
    }
}

fn stop(server: &mut Option<Server>) {
    if let Some(mut server) = server.take() {
        let _ = server.child.kill();
        let _ = server.child.wait();
    }
}

impl Server {
    fn start() -> Result<Self, String> {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new(crate::native::helper("openai_crypto")?);
        command
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(crate::process::CREATE_NO_WINDOW);
        crate::auth::isolate_environment(&mut command);
        let mut child = command.spawn().map_err(|_| FAILED.to_string())?;
        let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(FAILED.into());
        };
        let (sender, answers) = mpsc::channel();
        std::thread::spawn(move || {
            let mut output = BufReader::new(output);
            loop {
                let mut line = String::new();
                match output.by_ref().take(LINE_LIMIT).read_line(&mut line) {
                    Ok(_) if line.ends_with('\n') => {
                        if sender.send(line.trim_end().to_string()).is_err() {
                            break;
                        }
                    }
                    // End of output, an error or an overlong line.
                    _ => break,
                }
            }
        });
        Ok(Self {
            child,
            input: Some(input),
            answers,
        })
    }

    /// `Ok(None)` means the helper had exited before answering.
    fn exchange(
        &mut self,
        lines: &[String],
        cancel: &Cancellation,
    ) -> Result<Option<String>, String> {
        let request = format!("{}\n", lines.join("\n"));
        let started = Instant::now();
        // Pipe writes can block when a helper stops reading. Keep cancellation
        // and the operation deadline active while sending the request too.
        let mut input = self.input.take().ok_or(FAILED)?;
        let (sender, sent) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = input
                .write_all(request.as_bytes())
                .and_then(|()| input.flush());
            let _ = sender.send((input, result));
        });
        loop {
            if cancel.requested() {
                return Err("Operation cancelled".into());
            }
            let left = TIMEOUT.saturating_sub(started.elapsed());
            if left.is_zero() {
                return Err(FAILED.into());
            }
            match sent.recv_timeout(left.min(Duration::from_millis(50))) {
                Ok((input, result)) => {
                    self.input = Some(input);
                    if result.is_err() {
                        return Ok(None);
                    }
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(None),
            }
        }
        loop {
            if cancel.requested() {
                return Err("Operation cancelled".into());
            }
            let left = TIMEOUT.saturating_sub(started.elapsed());
            if left.is_zero() {
                return Err(FAILED.into());
            }
            match self
                .answers
                .recv_timeout(left.min(Duration::from_millis(50)))
            {
                Ok(answer) => return received(answer, cancel),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(None),
            }
        }
    }
}

fn received(answer: String, cancel: &Cancellation) -> Result<Option<String>, String> {
    if cancel.requested() {
        Err("Operation cancelled".into())
    } else {
        Ok(Some(answer))
    }
}

#[cfg(test)]
pub(super) fn process_id() -> Option<u32> {
    let server = SERVER.lock().unwrap_or_else(PoisonError::into_inner);
    server.as_ref().map(|server| server.child.id())
}

/// Ends the helper as its idle exit does, leaving the stale handle in place.
#[cfg(test)]
pub(super) fn exit_for_test() {
    let mut server = SERVER.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(server) = server.as_mut() {
        server.child.kill().unwrap();
        server.child.wait().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_waiter_does_not_wait_for_the_helper_lock() {
        let held = SERVER.lock().unwrap_or_else(PoisonError::into_inner);
        let cancel = Cancellation::default();
        let waiting_cancel = cancel.clone();
        let (sender, result) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let answer = request(&["random".into(), "1".into()], &waiting_cancel);
            sender.send(answer).unwrap();
        });
        std::thread::sleep(Duration::from_millis(50));
        cancel.cancel();
        assert_eq!(
            result.recv_timeout(Duration::from_secs(1)).unwrap(),
            Err("Operation cancelled".into())
        );
        drop(held);
        waiter.join().unwrap();
    }

    #[test]
    fn queued_reply_is_rejected_after_cancellation() {
        let cancel = Cancellation::default();
        let (sender, reply) = mpsc::channel();
        sender.send("true".to_string()).unwrap();
        cancel.cancel();
        assert_eq!(
            received(reply.recv().unwrap(), &cancel),
            Err("Operation cancelled".into())
        );
    }
}
