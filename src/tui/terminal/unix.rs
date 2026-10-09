mod mode;
use super::{Geometry, Input};
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub struct Terminal {
    pub input: Receiver<Input>,
    pub size: (usize, usize),
    stopped: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
    mode: mode::Mode,
}

impl Terminal {
    pub fn open(bash: &Path) -> Result<Self, String> {
        if std::env::var("TERM").is_ok_and(|term| term == "dumb") {
            return Err("The TUI needs a VT terminal. Use jecode --plain.".into());
        }
        let mut tty = File::open("/dev/tty").map_err(|error| {
            format!("Could not open terminal input: {error}. Use jecode --plain.")
        })?;
        let mode = mode::Mode::open(tty.try_clone().map_err(|e| e.to_string())?, bash)?;
        let size = size(&tty)?;
        let stopped = Arc::new(AtomicBool::new(false));
        let token = stopped.clone();
        let (sender, input) = mpsc::sync_channel(16);
        let reader = thread::spawn(move || {
            let mut buffer = [0u8; 16 * 1024];
            let mut measured = size;
            let mut checked = Instant::now();
            while !token.load(Ordering::Relaxed) {
                if checked.elapsed() >= Duration::from_millis(200) {
                    match self::size(&tty) {
                        Ok(next) if next != measured => {
                            measured = next;
                            if !send(&sender, &token, Input::Size(geometry(next))) {
                                break;
                            }
                        }
                        Ok(_) => {}
                        Err(error) => {
                            send(&sender, &token, Input::Error(error));
                            break;
                        }
                    }
                    checked = Instant::now();
                }
                match tty.read(&mut buffer) {
                    Ok(0) => thread::sleep(Duration::from_millis(1)),
                    Ok(count) => {
                        if !send(&sender, &token, Input::Bytes(buffer[..count].to_vec())) {
                            break;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => {
                        send(
                            &sender,
                            &token,
                            Input::Error(format!("Could not read terminal: {error}")),
                        );
                        break;
                    }
                }
            }
        });
        Ok(Self {
            input,
            size,
            stopped,
            reader: Some(reader),
            mode,
        })
    }

    pub fn take_initial(&mut self) -> Vec<Input> {
        vec![]
    }
    pub fn check(&mut self) -> Result<(), String> {
        self.mode.check()
    }
}

fn send(sender: &SyncSender<Input>, stopped: &AtomicBool, mut event: Input) -> bool {
    while !stopped.load(Ordering::Relaxed) {
        match sender.try_send(event) {
            Ok(()) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(pending)) => event = pending,
        }
        thread::sleep(Duration::from_millis(5));
    }
    false
}

fn geometry(size: (usize, usize)) -> Geometry {
    Geometry {
        width: size.0,
        height: size.1,
        row: 0,
        column: 0,
    }
}

fn size(tty: &File) -> Result<(usize, usize), String> {
    let value = mode::stty(tty, &["size"])?;
    let mut values = value.split_whitespace().map(str::parse::<usize>);
    match (values.next(), values.next(), values.next()) {
        (Some(Ok(height)), Some(Ok(width)), None) if height > 0 && width > 0 => Ok((width, height)),
        _ => Err("Could not measure terminal size. Use jecode --plain.".into()),
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Join reads before the guardian restores canonical input and the shell.
        self.stopped.store(true, Ordering::Relaxed);
        self.mode.stop_reads();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod native_tests;
