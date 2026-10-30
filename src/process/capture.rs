//! Output capture threads for supervised and leaf commands.

use std::io::Read;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Default)]
pub(super) struct Capture {
    pub(super) bytes: Vec<u8>,
    pub(super) truncated: bool,
    pub(super) error: Option<String>,
    pub(super) total_bytes: usize,
}

/// What a capture thread reports to the supervising loop, which waits on
/// these instead of polling.
pub(super) enum Event {
    /// Output for the observer, sent only by an observed stream.
    Chunk(Vec<u8>),
    /// The stream ended, failed, or its thread stopped.
    Finished,
}

/// The supervising loop's view of both capture threads.
pub(super) struct Events {
    receiver: mpsc::Receiver<Event>,
    chunks: Vec<Vec<u8>>,
    finished: usize,
}

impl Events {
    pub(super) fn new(receiver: mpsc::Receiver<Event>) -> Self {
        Self {
            receiver,
            chunks: Vec::new(),
            finished: 0,
        }
    }

    /// Takes every event already sent, without waiting.
    pub(super) fn collect(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            self.take(event);
        }
    }

    /// Waits up to `timeout` for the next event, so output and stream ends
    /// are handled as they arrive rather than at the next poll.
    pub(super) fn wait(&mut self, timeout: Duration) {
        match self.receiver.recv_timeout(timeout) {
            Ok(event) => self.take(event),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            // Both streams have ended; only the process state is left to poll.
            Err(mpsc::RecvTimeoutError::Disconnected) => thread::sleep(timeout),
        }
    }

    /// Observed output received so far, oldest first.
    pub(super) fn chunks(&mut self) -> std::vec::Drain<'_, Vec<u8>> {
        self.chunks.drain(..)
    }

    pub(super) fn readers_finished(&self) -> bool {
        self.finished >= 2
    }

    fn take(&mut self, event: Event) {
        match event {
            Event::Chunk(chunk) => self.chunks.push(chunk),
            Event::Finished => self.finished += 1,
        }
    }
}

/// Reports `Finished` when the capture thread ends, including by panic.
struct Finish(mpsc::Sender<Event>);

impl Drop for Finish {
    fn drop(&mut self) {
        let _ = self.0.send(Event::Finished);
    }
}

pub(super) fn capture(
    mut reader: impl Read + Send + 'static,
    output: Arc<Mutex<Capture>>,
    limit: usize,
    events: mpsc::Sender<Event>,
    observed: bool,
    tail: bool,
    mut spool: Option<crate::output::Spool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let finish = Finish(events);
        let mut buffer = [0; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    if let Some(spool) = &mut spool
                        && let Err(error) = spool.finish()
                    {
                        output.lock().unwrap().error = Some(error.to_string());
                    }
                    break;
                }
                Ok(length) => {
                    if let Some(spool) = &mut spool
                        && let Err(error) = spool.write(&buffer[..length])
                    {
                        output.lock().unwrap().error = Some(error.to_string());
                        break;
                    }
                    let mut output = output.lock().unwrap();
                    output.total_bytes = output.total_bytes.saturating_add(length);
                    let keep = length.min(limit.saturating_sub(output.bytes.len()));
                    if tail {
                        output.bytes.extend_from_slice(&buffer[..length]);
                        let drop = output.bytes.len().saturating_sub(limit);
                        output.bytes.drain(..drop);
                    } else {
                        output.bytes.extend_from_slice(&buffer[..keep]);
                    }
                    output.truncated |= keep < length;
                    drop(output);
                    // The retained preview limit must not truncate a streamed
                    // protocol consumed by the observer.
                    if observed {
                        let _ = finish.0.send(Event::Chunk(buffer[..length].to_vec()));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    output.lock().unwrap().error = Some(error.to_string());
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observer_receives_complete_output_beyond_the_retained_limit() {
        let bytes = vec![b'x'; 128 * 1024];
        let output = Arc::new(Mutex::new(Capture::default()));
        let (sender, receiver) = mpsc::channel();
        capture(
            std::io::Cursor::new(bytes.clone()),
            Arc::clone(&output),
            4096,
            sender,
            true,
            false,
            None,
        )
        .join()
        .unwrap();
        let mut observed = Vec::new();
        let mut finished = 0;
        for event in receiver {
            match event {
                Event::Chunk(chunk) => observed.extend(chunk),
                Event::Finished => finished += 1,
            }
        }
        assert_eq!(finished, 1);
        assert_eq!(observed.len(), bytes.len());
        assert!(observed == bytes);
        let output = output.lock().unwrap();
        assert_eq!(output.bytes.len(), 4096);
        assert_eq!(output.total_bytes, bytes.len());
        assert!(output.truncated);
    }
}
