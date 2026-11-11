use crate::json::{self, Value};
use crate::process::duplex::Duplex;
use std::io::{Read, Write};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Backlog of ordinary frames beyond one large reply. The reader cannot wait
/// for the receiver here: the server may be waiting for Jecode to write.
const MAX_QUEUED_FRAMES: usize = 4096;
const MAX_QUEUED_BYTES: usize = 16 * 1024 * 1024;
const QUEUE_OVERFLOW: &str = "LSP incoming queue overflow";
type WriteRequest = (Vec<u8>, SyncSender<Result<(), String>>);
/// A queued frame with the ordinary bytes it counts; `None` marks the one
/// frame admitted beyond the ordinary backlog.
type Incoming = Result<(Value, Option<usize>), String>;

#[derive(Default)]
struct QueueState {
    frames: usize,
    /// Bytes in queued frames other than the one large frame, which is any
    /// frame that did not fit the ordinary backlog when it arrived.
    bytes: usize,
    jumbo: bool,
    overflow: bool,
}

pub(super) struct Transport {
    process: Duplex,
    incoming: Receiver<Incoming>,
    queue: Arc<Mutex<QueueState>>,
    writer: SyncSender<WriteRequest>,
}

impl Transport {
    pub fn spawn(
        command: &mut Command,
        cancel: &crate::cancel::Cancellation,
    ) -> Result<Self, String> {
        let mut process = Duplex::spawn_cancellable(command, cancel)?;
        let mut stdout = process.stdout.take().ok_or("LSP stdout unavailable")?;
        let mut stderr = process.stderr.take().ok_or("LSP stderr unavailable")?;
        let mut stdin = process.stdin.take().ok_or("LSP stdin unavailable")?;
        let (writer, writes) = mpsc::sync_channel::<WriteRequest>(16);
        thread::spawn(move || {
            while let Ok((bytes, ack)) = writes.recv() {
                let result = stdin
                    .write_all(&bytes)
                    .and_then(|()| stdin.flush())
                    .map_err(|e| e.to_string());
                let failed = result.is_err();
                let _ = ack.try_send(result);
                if failed {
                    break;
                }
            }
        });
        // The reader must not block when the client is writing to a server that also writes.
        // Account for queued frames instead; an excessive burst retires the instance.
        let (sender, incoming) = mpsc::channel();
        let queue = Arc::new(Mutex::new(QueueState::default()));
        let reader_queue = Arc::clone(&queue);
        thread::spawn(move || {
            loop {
                let result = read_frame(&mut stdout);
                match result {
                    Ok(frame) => {
                        if !queue_frame(&sender, &reader_queue, frame) {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                }
            }
        });
        thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(count) = stderr.read(&mut buffer) {
                if count == 0 {
                    break;
                }
            }
        });
        Ok(Self {
            process,
            incoming,
            queue,
            writer,
        })
    }

    pub fn id(&self) -> u32 {
        self.process.id()
    }

    pub fn send(&mut self, message: Value) -> Result<(), String> {
        let json = message.encode();
        let mut bytes = format!("Content-Length: {}\r\n\r\n", json.len()).into_bytes();
        bytes.extend_from_slice(json.as_bytes());
        let (ack, result) = mpsc::sync_channel(1);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut pending = Some((bytes, ack));
        loop {
            match self.writer.try_send(pending.take().expect("pending write")) {
                Ok(()) => break,
                Err(TrySendError::Full(item)) => pending = Some(item),
                Err(TrySendError::Disconnected(_)) => return Err("LSP writer exited".into()),
            }
            if std::time::Instant::now() >= deadline {
                let _ = self.process.kill_tree();
                return Err("LSP writer queue timed out".into());
            }
            thread::sleep(Duration::from_millis(5));
        }
        match result.recv_timeout(Duration::from_secs(2)) {
            Ok(result) => result,
            Err(_) => {
                let _ = self.process.kill_tree();
                Err("LSP write timed out".into())
            }
        }
    }

    pub fn recv(&self, timeout: Duration) -> Result<Option<Value>, String> {
        receive_frame(&self.incoming, &self.queue, timeout)
    }

    #[cfg(test)]
    pub(super) fn queued_frames(&self) -> usize {
        self.queue.lock().expect("LSP incoming queue").frames
    }

    pub fn stop(&mut self) {
        if !self
            .process
            .wait_timeout(Duration::from_millis(200))
            .unwrap_or(false)
        {
            let _ = self.process.kill_tree();
            let _ = self.process.wait_timeout(Duration::from_millis(500));
        }
    }
}

fn queue_frame(
    sender: &mpsc::Sender<Incoming>,
    queue: &Mutex<QueueState>,
    frame: (Value, usize),
) -> bool {
    let mut state = queue.lock().expect("LSP incoming queue");
    let (value, bytes) = frame;
    let ordinary = state.bytes.saturating_add(bytes) <= MAX_QUEUED_BYTES;
    if state.frames >= MAX_QUEUED_FRAMES || !ordinary && state.jumbo {
        state.overflow = true;
        return false;
    }
    state.frames += 1;
    let counted = if ordinary {
        state.bytes += bytes;
        Some(bytes)
    } else {
        state.jumbo = true;
        None
    };
    drop(state);
    sender.send(Ok((value, counted))).is_ok()
}

fn receive_frame(
    incoming: &Receiver<Incoming>,
    queue: &Mutex<QueueState>,
    timeout: Duration,
) -> Result<Option<Value>, String> {
    if queue.lock().expect("LSP incoming queue").overflow {
        return Err(QUEUE_OVERFLOW.into());
    }
    match incoming.recv_timeout(timeout) {
        Ok(Ok((value, bytes))) => {
            let mut state = queue.lock().expect("LSP incoming queue");
            state.frames -= 1;
            match bytes {
                Some(bytes) => state.bytes -= bytes,
                None => state.jumbo = false,
            }
            if state.overflow {
                Err(QUEUE_OVERFLOW.into())
            } else {
                Ok(Some(value))
            }
        }
        Ok(Err(error)) => Err(error),
        Err(RecvTimeoutError::Timeout) => Ok(None),
        Err(RecvTimeoutError::Disconnected) => Err("Language server exited".into()),
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        self.stop();
    }
}

fn read_frame(input: &mut impl Read) -> Result<(Value, usize), String> {
    let mut header = Vec::new();
    let mut byte = [0];
    while !header.ends_with(b"\r\n\r\n") {
        input
            .read_exact(&mut byte)
            .map_err(|e| format!("LSP stream ended: {e}"))?;
        header.push(byte[0]);
        if header.len() > 8192 {
            return Err("LSP header exceeds 8 KiB".into());
        }
    }
    let text = std::str::from_utf8(&header).map_err(|_| "Invalid LSP header")?;
    let mut length = None;
    for line in text.split("\r\n") {
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("Content-Length")
        {
            if length.is_some() {
                return Err("Duplicate LSP Content-Length".into());
            }
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "Invalid LSP Content-Length")?,
            );
        }
    }
    let length = length.ok_or("Missing LSP Content-Length")?;
    // Memory follows the bytes that arrive, not the length a header claims.
    let mut body = Vec::new();
    input
        .by_ref()
        .take(length as u64)
        .read_to_end(&mut body)
        .map_err(|e| format!("Incomplete LSP frame: {e}"))?;
    if body.len() < length {
        return Err("Incomplete LSP frame: stream ended".into());
    }
    let value = json::parse(std::str::from_utf8(&body).map_err(|_| "LSP frame is not UTF-8")?)?;
    Ok((value, length))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_handles_header_case_and_unicode() {
        let mut input = b"content-length: 15\r\n\r\n{\"x\":\"\xc3\xa9\"}".as_slice();
        assert!(read_frame(&mut input).is_err());
        let mut input = b"content-length: 10\r\n\r\n{\"x\":\"\xc3\xa9\"}".as_slice();
        assert_eq!(
            read_frame(&mut input)
                .unwrap()
                .0
                .get("x")
                .and_then(Value::as_str),
            Some("é")
        );
    }

    #[test]
    fn paused_receiver_reports_a_bounded_frame_overflow() {
        let (sender, incoming) = mpsc::channel();
        let queue = Mutex::new(QueueState::default());
        for _ in 0..MAX_QUEUED_FRAMES {
            assert!(queue_frame(&sender, &queue, (Value::Null, 1)));
        }
        assert!(!queue_frame(&sender, &queue, (Value::Null, 1)));
        assert_eq!(
            receive_frame(&incoming, &queue, Duration::ZERO).unwrap_err(),
            QUEUE_OVERFLOW
        );
    }

    #[test]
    fn paused_receiver_reports_a_bounded_byte_overflow() {
        let (sender, incoming) = mpsc::channel();
        let queue = Mutex::new(QueueState::default());
        for _ in 0..2 {
            assert!(queue_frame(
                &sender,
                &queue,
                (Value::Null, MAX_QUEUED_BYTES / 2)
            ));
        }
        // One frame beyond the full backlog is admitted; the next overflows.
        assert!(queue_frame(&sender, &queue, (Value::Null, 1)));
        assert!(!queue_frame(&sender, &queue, (Value::Null, 1)));
        assert_eq!(
            receive_frame(&incoming, &queue, Duration::ZERO).unwrap_err(),
            QUEUE_OVERFLOW
        );
    }

    #[test]
    fn one_frame_past_the_former_8_mib_is_read_and_queued() {
        let text = "x".repeat(10 * 1024 * 1024);
        let json = Value::object([("text", Value::string(&text))]).encode();
        let mut stream = format!("Content-Length: {}\r\n\r\n", json.len()).into_bytes();
        stream.extend_from_slice(json.as_bytes());
        let (value, length) = read_frame(&mut stream.as_slice()).unwrap();
        assert_eq!(
            value.get("text").and_then(Value::as_str),
            Some(text.as_str())
        );
        // The frame enters and leaves the receive queue.
        let (sender, incoming) = mpsc::channel();
        let queue = Mutex::new(QueueState::default());
        assert!(queue_frame(&sender, &queue, (value, length)));
        assert!(receive_frame(&incoming, &queue, Duration::ZERO).is_ok());
        // A header claiming more than arrives fails without allocating it.
        let mut short = b"Content-Length: 99999999999\r\n\r\n{}".as_slice();
        assert!(read_frame(&mut short).unwrap_err().contains("Incomplete"));
    }

    #[test]
    fn a_reply_that_overflows_the_backlog_is_the_one_large_frame_whatever_its_size() {
        let (sender, incoming) = mpsc::channel();
        let queue = Mutex::new(QueueState::default());
        let mib = 1024 * 1024;
        assert!(queue_frame(&sender, &queue, (Value::Null, 3 * mib)));
        assert!(queue_frame(
            &sender,
            &queue,
            (Value::string("reply"), 14 * mib)
        ));
        assert!(receive_frame(&incoming, &queue, Duration::ZERO).is_ok());
        assert_eq!(
            receive_frame(&incoming, &queue, Duration::ZERO).unwrap(),
            Some(Value::string("reply"))
        );
        let state = queue.lock().unwrap();
        assert_eq!((state.frames, state.bytes, state.jumbo), (0, 0, false));
    }

    #[test]
    fn large_reply_and_following_notification_share_a_bounded_queue() {
        let (sender, incoming) = mpsc::channel();
        let queue = Mutex::new(QueueState::default());
        let large = MAX_QUEUED_BYTES + 1;
        assert!(queue_frame(
            &sender,
            &queue,
            (Value::string("reply"), large)
        ));
        assert!(queue_frame(
            &sender,
            &queue,
            (Value::string("notification"), 1)
        ));
        assert_eq!(
            receive_frame(&incoming, &queue, Duration::ZERO).unwrap(),
            Some(Value::string("reply"))
        );
        assert_eq!(
            receive_frame(&incoming, &queue, Duration::ZERO).unwrap(),
            Some(Value::string("notification"))
        );
        assert!(queue_frame(&sender, &queue, (Value::Null, large)));
        assert!(queue_frame(
            &sender,
            &queue,
            (Value::Null, MAX_QUEUED_BYTES)
        ));
        assert!(!queue_frame(&sender, &queue, (Value::Null, 1)));
        assert_eq!(
            receive_frame(&incoming, &queue, Duration::ZERO).unwrap_err(),
            QUEUE_OVERFLOW
        );
    }
}
