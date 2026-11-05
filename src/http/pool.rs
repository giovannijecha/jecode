//! Reusable native HTTP adapters, one per request in flight. Every provider, web
//! and MCP request skips process startup and keeps its connections open. An
//! adapter stopped by cancellation, a deadline or its observer is discarded.

use super::adapter::{Adapter, Frame};
use crate::cancel::Cancellation;
use crate::process::Output;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// More only happens during a burst of parallel collaborators, and each idle
/// adapter is a process.
const MAX_IDLE: usize = 4;
/// Frames wake the loop at once; the interval bounds how late cancellation and
/// deadlines are noticed.
const POLL: Duration = Duration::from_millis(10);

static SHARED: Pool = Pool::new();

pub(super) struct Pool {
    idle: Mutex<Vec<Adapter>>,
}

pub(super) struct Limits {
    pub timeout: Option<Duration>,
    pub idle: Option<Duration>,
    pub output: usize,
}

enum Outcome {
    /// The adapter ended before it had the request, as an idle exit does.
    Unstarted,
    Reusable(Output),
    Discard(Result<Output, String>),
}

pub(super) fn run(
    input: Vec<u8>,
    limits: Limits,
    cancellation: &Cancellation,
    observer: &mut dyn FnMut(&[u8]) -> Result<(), String>,
) -> Result<Output, String> {
    SHARED.run(input, limits, cancellation, observer)
}

impl Pool {
    pub(super) const fn new() -> Self {
        Self {
            idle: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn run(
        &self,
        input: Vec<u8>,
        limits: Limits,
        cancellation: &Cancellation,
        observer: &mut dyn FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<Output, String> {
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        let input = Arc::new(input);
        // Sending again is safe: an adapter that never accepted sent nothing.
        for _ in 0..2 {
            let mut adapter = self.acquire()?;
            match exchange(&mut adapter, &input, &limits, cancellation, observer) {
                Outcome::Unstarted => adapter.stop(),
                Outcome::Reusable(output) => {
                    self.release(adapter);
                    return Ok(output);
                }
                Outcome::Discard(result) => {
                    adapter.stop();
                    return result;
                }
            }
        }
        Err("The native HTTP adapter stopped before accepting the request".into())
    }

    fn acquire(&self) -> Result<Adapter, String> {
        let mut idle = self.idle.lock().unwrap_or_else(PoisonError::into_inner);
        // The newest adapter holds the connections most likely to still be open.
        while let Some(mut adapter) = idle.pop() {
            if adapter.alive() {
                return Ok(adapter);
            }
            adapter.stop();
        }
        drop(idle);
        Adapter::start(&[])
    }

    fn release(&self, adapter: Adapter) {
        let mut idle = self.idle.lock().unwrap_or_else(PoisonError::into_inner);
        if idle.len() < MAX_IDLE {
            idle.push(adapter);
        } else {
            drop(idle);
            adapter.stop();
        }
    }

    #[cfg(test)]
    pub(super) fn idle_ids(&self) -> Vec<u32> {
        self.idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(Adapter::id)
            .collect()
    }

    /// Ends the idle adapters as their idle exit does, leaving stale handles.
    #[cfg(test)]
    pub(super) fn exit_idle_for_test(&self) {
        for adapter in self
            .idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter_mut()
        {
            adapter.exit_for_test();
        }
    }
}

#[cfg(test)]
impl Drop for Pool {
    fn drop(&mut self) {
        let idle = self.idle.get_mut().unwrap_or_else(PoisonError::into_inner);
        for adapter in idle.drain(..) {
            adapter.stop();
        }
    }
}

fn exchange(
    adapter: &mut Adapter,
    input: &Arc<Vec<u8>>,
    limits: &Limits,
    cancellation: &Cancellation,
    observer: &mut dyn FnMut(&[u8]) -> Result<(), String>,
) -> Outcome {
    let Some(delivery) = adapter.deliver(Arc::clone(input)) else {
        return Outcome::Unstarted;
    };
    let started = Instant::now();
    let mut last_activity = started;
    let mut accepted = false;
    let mut report = Report::new(limits.output);
    loop {
        if cancellation.requested() {
            report.cancelled = true;
            return Outcome::Discard(Ok(report.finish(None, Vec::new())));
        }
        if limits
            .timeout
            .is_some_and(|limit| started.elapsed() >= limit)
            || limits
                .idle
                .is_some_and(|limit| last_activity.elapsed() >= limit)
        {
            report.timed_out = true;
            return Outcome::Discard(Ok(report.finish(None, Vec::new())));
        }
        match adapter.frames.recv_timeout(POLL) {
            Ok(Frame::Accepted) => accepted = true,
            Ok(Frame::Data(bytes)) => {
                last_activity = Instant::now();
                report.keep(&bytes);
                if let Err(error) = observer(&bytes) {
                    return Outcome::Discard(Err(error));
                }
            }
            Ok(Frame::End(message)) => {
                // The whole request arrived before `Accepted`, so its delivery
                // is complete unless the adapter broke the pipe.
                let healthy = adapter.recover(delivery);
                let code = i32::from(!message.is_empty());
                let output = report.finish(Some(code), message);
                return if healthy {
                    Outcome::Reusable(output)
                } else {
                    Outcome::Discard(Ok(output))
                };
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) if !accepted => return Outcome::Unstarted,
            Err(RecvTimeoutError::Disconnected) => {
                let message = b"Native HTTP adapter stopped during the request".to_vec();
                return Outcome::Discard(Ok(report.finish(Some(1), message)));
            }
        }
    }
}

/// The bounded `Output` of one exchange; the observer sees every byte.
struct Report {
    retained: Vec<u8>,
    limit: usize,
    total: usize,
    truncated: bool,
    timed_out: bool,
    cancelled: bool,
}

impl Report {
    fn new(limit: usize) -> Self {
        Self {
            retained: Vec::new(),
            limit,
            total: 0,
            truncated: false,
            timed_out: false,
            cancelled: false,
        }
    }

    fn keep(&mut self, bytes: &[u8]) {
        self.total = self.total.saturating_add(bytes.len());
        let keep = bytes
            .len()
            .min(self.limit.saturating_sub(self.retained.len()));
        self.retained.extend_from_slice(&bytes[..keep]);
        self.truncated |= keep < bytes.len();
    }

    fn finish(self, exit_code: Option<i32>, stderr: Vec<u8>) -> Output {
        Output {
            stdout: self.retained,
            stderr_bytes: stderr.len(),
            stderr,
            exit_code,
            input_error: None,
            timed_out: self.timed_out,
            stdout_bytes: self.total,
            stdout_truncated: self.truncated,
            cancelled: self.cancelled,
        }
    }
}
