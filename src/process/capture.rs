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
                    drop(output);
                    // The retained preview limit must not truncate a streamed
                    // protocol consumed by the observer.
                    if let Some(chunks) = &chunks {
                        let _ = chunks.send(buffer[..length].to_vec());
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
            Some(sender),
            false,
            None,
        )
        .join()
        .unwrap();
        let observed: Vec<_> = receiver.into_iter().flatten().collect();
        assert_eq!(observed.len(), bytes.len());
        assert!(observed == bytes);
        let output = output.lock().unwrap();
        assert_eq!(output.bytes.len(), 4096);
        assert_eq!(output.total_bytes, bytes.len());
        assert!(output.truncated);
    }
}
