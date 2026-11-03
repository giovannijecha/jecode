use crate::json::{self, Value};
use crate::process::duplex::Duplex;
use std::io::{Read, Write};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::thread;
use std::time::Duration;

const MAX_FRAME: usize = 8 * 1024 * 1024;
type WriteRequest = (Vec<u8>, SyncSender<Result<(), String>>);

pub(super) struct Transport {
    process: Duplex,
    incoming: Receiver<Result<Value, String>>,
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
        // Unbounded: a server may emit bursts while the client is busy, and blocking here
        // could deadlock a single-threaded server that is waiting to write while we write.
        let (sender, incoming) = mpsc::channel();
        thread::spawn(move || {
            loop {
                let result = read_frame(&mut stdout);
                let done = result.is_err();
                if sender.send(result).is_err() || done {
                    break;
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
            writer,
        })
    }

    pub fn id(&self) -> u32 {
        self.process.id()
    }

    pub fn send(&mut self, message: Value) -> Result<(), String> {
        let json = message.encode();
        if json.len() > MAX_FRAME {
            return Err("LSP message exceeds 8 MiB".into());
        }
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
        match self.incoming.recv_timeout(timeout) {
            Ok(result) => result.map(Some),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err("Language server exited".into()),
        }
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

impl Drop for Transport {
    fn drop(&mut self) {
        self.stop();
    }
}

fn read_frame(input: &mut impl Read) -> Result<Value, String> {
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
    if length > MAX_FRAME {
        return Err("LSP frame exceeds 8 MiB".into());
    }
    let mut body = vec![0; length];
    input
        .read_exact(&mut body)
        .map_err(|e| format!("Incomplete LSP frame: {e}"))?;
    json::parse(std::str::from_utf8(&body).map_err(|_| "LSP frame is not UTF-8")?)
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
                .get("x")
                .and_then(Value::as_str),
            Some("é")
        );
    }
}
