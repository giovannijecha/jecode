//! Output capture threads for supervised and leaf commands.

use std::io::Read;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};

#[derive(Default)]
pub(super) struct Capture {
    pub(super) bytes: Vec<u8>,
    pub(super) truncated: bool,
    pub(super) error: Option<String>,
    pub(super) total_bytes: usize,
}

pub(super) fn capture(
    mut reader: impl Read + Send + 'static,
    output: Arc<Mutex<Capture>>,
    limit: usize,
    chunks: Option<mpsc::Sender<Vec<u8>>>,
    tail: bool,
    mut spool: Option<crate::output::Spool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
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
                    if keep > 0
                        && let Some(chunks) = &chunks
                    {
                        let _ = chunks.send(buffer[..keep].to_vec());
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
