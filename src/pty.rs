//! Native pseudo-terminals supervised like other conversation processes.
use crate::{cancel::Cancellation, process};
use std::fs::File;
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
use windows as platform;

pub(crate) const MAX_INPUT: usize = 64 * 1024;
const DELIVERY: Duration = Duration::from_secs(2);

struct Message {
    bytes: Vec<u8>,
    receipt: Option<mpsc::SyncSender<Result<(), String>>>,
}

pub(crate) struct Control {
    sender: Mutex<Option<mpsc::SyncSender<Message>>>,
    controls: Mutex<Option<mpsc::SyncSender<Message>>>,
    failure: Arc<Mutex<Option<String>>>,
    closed: Arc<AtomicBool>,
    size: Mutex<(u16, u16)>,
    #[cfg(unix)]
    generation: Mutex<u64>,
    _endpoint: Endpoint,
}

impl Drop for Control {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

struct Endpoint {
    name: String,
    #[cfg(unix)]
    directory: std::path::PathBuf,
}

impl Endpoint {
    fn new() -> Result<Self, String> {
        let id = crate::sessions::identifier();
        #[cfg(windows)]
        {
            Ok(Self {
                name: format!("jecode-pty-{id}"),
            })
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let directory = std::env::temp_dir().join(format!("jecode-pty-{id}"));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .map_err(|e| format!("Could not create terminal endpoint: {e}"))?;
            Ok(Self {
                name: directory.join("control").to_string_lossy().into_owned(),
                directory,
            })
        }
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.name);
            let _ = std::fs::remove_dir(&self.directory);
        }
    }
}

pub(crate) fn start(
    command: &Command,
    size: (u16, u16),
    cancellation: &Cancellation,
    spools: [crate::output::Spool; 2],
) -> Result<(process::Started, Arc<Control>), process::RunError> {
    let before = |message| process::RunError {
        started: false,
        message,
    };
    let endpoint = Endpoint::new().map_err(before)?;
    let mut bridge =
        platform::command(command, &endpoint.name, size.0, size.1, cancellation).map_err(before)?;
    let started = process::start_logged(&mut bridge, cancellation, spools)?;
    #[cfg(windows)]
    let connection = platform::connect(&endpoint.name, started.bridge_pid(), cancellation)
        .and_then(|input| {
            platform::connect_control(&endpoint.name, started.bridge_pid(), cancellation)
                .map(|control| (input, control))
        });
    #[cfg(unix)]
    let connection = platform::connect(&endpoint.name, cancellation).and_then(|input| {
        platform::connect_control(&endpoint.name, cancellation).map(|control| (input, control))
    });
    let files = match connection {
        Ok(files) => files,
        Err(error) => {
            let stop = Cancellation::default();
            stop.cancel();
            let cleanup = started.supervise(None, &stop, &mut |_| Ok(()));
            let message = match cleanup {
                Ok(_) => error,
                Err(cleanup) => format!(
                    "{error}; terminal cleanup could not be confirmed: {}",
                    cleanup.message
                ),
            };
            return Err(process::RunError {
                started: true,
                message,
            });
        }
    };
    let (sender, receiver) = mpsc::sync_channel::<Message>(16);
    let (controls, control_receiver) = mpsc::sync_channel::<Message>(16);
    let failure = Arc::new(Mutex::new(None));
    let closed = Arc::new(AtomicBool::new(false));
    let failed = Arc::clone(&failure);
    let shutdown = Arc::clone(&closed);
    thread::spawn(move || writer(files.0, receiver, failed, shutdown));
    let failed = Arc::clone(&failure);
    let shutdown = Arc::clone(&closed);
    thread::spawn(move || writer(files.1, control_receiver, failed, shutdown));
    Ok((
        started,
        Arc::new(Control {
            sender: Mutex::new(Some(sender)),
            controls: Mutex::new(Some(controls)),
            failure,
            closed,
            size: Mutex::new(size),
            #[cfg(unix)]
            generation: Mutex::new(0),
            _endpoint: endpoint,
        }),
    ))
}

fn writer(
    mut file: File,
    receiver: mpsc::Receiver<Message>,
    failure: Arc<Mutex<Option<String>>>,
    closed: Arc<AtomicBool>,
) {
    for message in receiver {
        if closed.load(Ordering::Relaxed) {
            break;
        }
        let mut bytes = message.bytes.as_slice();
        let result = (|| {
            while !bytes.is_empty() {
                if closed.load(Ordering::Relaxed) {
                    return Err("Terminal has ended".into());
                }
                match file.write(bytes) {
                    Ok(0) => return Err("Terminal control channel closed".to_string()),
                    Ok(n) => bytes = &bytes[n..],
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => return Err(format!("Terminal input failed: {error}")),
                }
            }
            Ok(())
        })();
        if let Err(error) = &result {
            *failure.lock().unwrap() = Some(error.clone());
        }
        let failed = result.is_err();
        if let Some(receipt) = message.receipt {
            let _ = receipt.send(result);
        }
        if failed {
            break;
        }
    }
}

fn frame(op: u8, bytes: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(bytes.len() + 5);
    packet.push(op);
    packet.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    packet.extend_from_slice(bytes);
    packet
}

impl Control {
    #[cfg(unix)]
    fn enqueue_input(
        &self,
        bytes: &[u8],
        receipt: Option<mpsc::SyncSender<Result<(), String>>>,
    ) -> Result<(), String> {
        if bytes.len() > MAX_INPUT {
            return Err(format!(
                "Terminal input must be at most {MAX_INPUT} bytes per call"
            ));
        }
        let generation = self.generation.lock().unwrap();
        let mut packet = Vec::new();
        for chunk in bytes.chunks(4096) {
            let mut payload = Vec::with_capacity(chunk.len() + 8);
            payload.extend_from_slice(&generation.to_le_bytes());
            payload.extend_from_slice(chunk);
            packet.extend(frame(1, &payload));
        }
        self.enqueue(packet, false, receipt)
    }

    #[cfg(unix)]
    fn enqueue_interrupt(
        &self,
        receipt: Option<mpsc::SyncSender<Result<(), String>>>,
    ) -> Result<(), String> {
        let mut generation = self.generation.lock().unwrap();
        let next = generation
            .checked_add(1)
            .ok_or("Terminal interrupt sequence exhausted")?;
        self.enqueue(frame(3, &next.to_le_bytes()), true, receipt)?;
        *generation = next;
        Ok(())
    }

    pub fn size(&self) -> (u16, u16) {
        *self.size.lock().unwrap()
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.sender.lock().unwrap().take();
        self.controls.lock().unwrap().take();
    }

    fn enqueue(
        &self,
        bytes: Vec<u8>,
        control: bool,
        receipt: Option<mpsc::SyncSender<Result<(), String>>>,
    ) -> Result<(), String> {
        if let Some(error) = &*self.failure.lock().unwrap() {
            return Err(error.clone());
        }
        (if control {
            &self.controls
        } else {
            &self.sender
        })
        .lock()
        .unwrap()
        .as_ref()
        .ok_or("Terminal has ended")?
        .try_send(Message { bytes, receipt })
        .map_err(|error| match error {
            mpsc::TrySendError::Full(_) => {
                "Terminal input queue is full; poll before sending more".into()
            }
            mpsc::TrySendError::Disconnected(_) => "Terminal control channel closed".into(),
        })
    }

    fn deliver(
        &self,
        bytes: Vec<u8>,
        control: bool,
        cancellation: &Cancellation,
    ) -> Result<(), String> {
        self.deliver_with(cancellation, |receipt| {
            self.enqueue(bytes, control, receipt)
        })
    }

    fn deliver_with(
        &self,
        cancellation: &Cancellation,
        enqueue: impl FnOnce(Option<mpsc::SyncSender<Result<(), String>>>) -> Result<(), String>,
    ) -> Result<(), String> {
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        enqueue(Some(sender))?;
        let deadline = Instant::now() + DELIVERY;
        loop {
            match receiver.recv_timeout(Duration::from_millis(20)) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Terminal input delivery failed".into());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if cancellation.requested() || Instant::now() >= deadline {
                return Err("Terminal input delivery was not confirmed; input may still be delivered. Poll before retrying.".into());
            }
        }
    }

    #[cfg(windows)]
    fn input_packet(bytes: &[u8]) -> Result<Vec<u8>, String> {
        if bytes.len() > MAX_INPUT {
            return Err(format!(
                "Terminal input must be at most {MAX_INPUT} bytes per call"
            ));
        }
        Ok(bytes
            .chunks(4096)
            .flat_map(|chunk| frame(1, chunk))
            .collect())
    }

    pub fn write(&self, bytes: &[u8], cancellation: &Cancellation) -> Result<(), String> {
        #[cfg(unix)]
        return self.deliver_with(cancellation, |receipt| self.enqueue_input(bytes, receipt));
        #[cfg(windows)]
        self.deliver(Self::input_packet(bytes)?, false, cancellation)
    }

    /// Local user input is not a tool call or a conversation message.
    pub fn write_direct(&self, bytes: &[u8]) -> Result<(), String> {
        #[cfg(unix)]
        return self.enqueue_input(bytes, None);
        #[cfg(windows)]
        self.enqueue(Self::input_packet(bytes)?, false, None)
    }

    pub fn resize(&self, size: (u16, u16), cancellation: &Cancellation) -> Result<(), String> {
        validate_size(size)?;
        let payload = [size.0.to_le_bytes(), size.1.to_le_bytes()].concat();
        self.deliver(frame(2, &payload), true, cancellation)?;
        *self.size.lock().unwrap() = size;
        Ok(())
    }

    pub fn resize_direct(&self, size: (u16, u16)) -> Result<(), String> {
        validate_size(size)?;
        let payload = [size.0.to_le_bytes(), size.1.to_le_bytes()].concat();
        self.enqueue(frame(2, &payload), true, None)?;
        *self.size.lock().unwrap() = size;
        Ok(())
    }

    pub fn interrupt(&self, cancellation: &Cancellation) -> Result<(), String> {
        #[cfg(unix)]
        return self.deliver_with(cancellation, |receipt| self.enqueue_interrupt(receipt));
        #[cfg(windows)]
        self.deliver(frame(3, &[]), true, cancellation)
    }

    pub fn interrupt_direct(&self) -> Result<(), String> {
        #[cfg(unix)]
        return self.enqueue_interrupt(None);
        #[cfg(windows)]
        self.enqueue(frame(3, &[]), true, None)
    }
}

pub(crate) fn validate_size(size: (u16, u16)) -> Result<(), String> {
    if !(1..=1000).contains(&size.0) || !(1..=1000).contains(&size.1) {
        return Err("Terminal dimensions must be from 1 to 1000".into());
    }
    Ok(())
}

pub(crate) fn size(arguments: &crate::json::Value) -> Result<(u16, u16), String> {
    let dimension = |key, default| match arguments.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_usize()
            .filter(|n| (1..=1000).contains(n))
            .map(|n| n as u16)
            .ok_or_else(|| format!("{key} must be an integer from 1 to 1000")),
    };
    Ok((dimension("cols", 80)?, dimension("rows", 24)?))
}

pub(crate) fn key(name: &str) -> Result<&'static [u8], String> {
    Ok(match name {
        "Enter" => b"\r",
        "Tab" => b"\t",
        "Escape" => b"\x1b",
        "Backspace" => b"\x7f",
        "Up" => b"\x1b[A",
        "Down" => b"\x1b[B",
        "Right" => b"\x1b[C",
        "Left" => b"\x1b[D",
        "Home" => b"\x1b[H",
        "End" => b"\x1b[F",
        "Delete" => b"\x1b[3~",
        "Ctrl+C" => b"\x03",
        "Ctrl+D" => b"\x04",
        "Ctrl+Z" => b"\x1a",
        _ => return Err(format!("Unknown terminal key {name}")),
    })
}

#[cfg(test)]
#[path = "pty/tests.rs"]
mod tests;
