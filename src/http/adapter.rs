//! A long-lived native HTTP adapter and its framed output. Requests reach it on
//! stdin, never in arguments or files.

use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

/// A socket frame holds one whole message; a longer one means a broken stream.
const FRAME_LIMIT: usize = 64 * 1024 * 1024;

pub(crate) enum Frame {
    /// The adapter received the whole request and has sent nothing yet.
    Accepted,
    Data(Vec<u8>),
    /// The response ended: empty on success, otherwise the failure message.
    End(Vec<u8>),
    /// The transport failed; the caller may retry the request.
    RetryableEnd(Vec<u8>),
}

pub(crate) struct Adapter {
    child: Child,
    input: Option<ChildStdin>,
    pub(crate) frames: Receiver<Frame>,
}

/// Delivers one input on its own thread, because a pipe write can block while
/// the adapter is busy and cancellation must stay responsive.
pub(crate) struct Delivery(Receiver<io::Result<ChildStdin>>);

impl Adapter {
    pub(crate) fn start(arguments: &[&str]) -> Result<Self, String> {
        let executable = crate::native::helper("http")?;
        let mut command = Command::new(&executable);
        command
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        command.creation_flags(crate::process::CREATE_NO_WINDOW);
        // Its own group: Ctrl+C at the terminal must not end a shared adapter.
        #[cfg(unix)]
        command.process_group(0);
        // It outlives the commands around it and must not pin their folder.
        if let Some(folder) = executable.parent() {
            command.current_dir(folder);
        }
        crate::auth::isolate_environment(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| format!("Could not start the native HTTP adapter: {error}"))?;
        let (Some(input), Some(mut output)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Could not connect the native HTTP adapter".into());
        };
        let (sender, frames) = mpsc::channel();
        std::thread::spawn(move || {
            while let Some(frame) = read_frame(&mut output) {
                if sender.send(frame).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input: Some(input),
            frames,
        })
    }

    pub(crate) fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// `None` when an earlier delivery failed and the pipe was not recovered.
    pub(crate) fn deliver(&mut self, bytes: Arc<Vec<u8>>) -> Option<Delivery> {
        let mut input = self.input.take()?;
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = input.write_all(&bytes).and_then(|()| input.flush());
            let _ = sender.send(result.map(|()| input));
        });
        Some(Delivery(receiver))
    }

    /// Takes the pipe back once its input was delivered in full.
    pub(crate) fn recover(&mut self, delivery: Delivery) -> bool {
        match delivery.0.recv() {
            Ok(Ok(input)) => {
                self.input = Some(input);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    #[cfg(test)]
    pub(crate) fn id(&self) -> u32 {
        self.child.id()
    }

    /// What the parent exiting does to the adapter.
    #[cfg(test)]
    pub(crate) fn close_input_for_test(&mut self) {
        self.input = None;
    }

    #[cfg(test)]
    pub(crate) fn exit_for_test(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

fn read_frame(output: &mut impl Read) -> Option<Frame> {
    let mut head = [0; 5];
    output.read_exact(&mut head).ok()?;
    let length = u32::from_le_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if length > FRAME_LIMIT {
        return None;
    }
    let mut payload = vec![0; length];
    output.read_exact(&mut payload).ok()?;
    match head[0] {
        b'A' => Some(Frame::Accepted),
        b'D' => Some(Frame::Data(payload)),
        b'E' => Some(Frame::End(payload)),
        b'T' => Some(Frame::RetryableEnd(payload)),
        _ => None,
    }
}
