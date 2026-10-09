use super::before_start;
use crate::{cancel::Cancellation, process::RunError};
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// OS open(2) constants, not an implementation imported from another library.
#[cfg(target_os = "linux")]
const NONBLOCK: i32 = 0x800;
#[cfg(target_os = "macos")]
const NONBLOCK: i32 = 0x4;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("The Unix process boundary supports Linux and macOS");

static NEXT: AtomicU64 = AtomicU64::new(0);

pub(super) struct Pipe {
    directory: PathBuf,
    path: PathBuf,
    reader: Option<File>,
    writer: Option<File>,
}

impl Pipe {
    pub(super) fn open() -> Result<Self, RunError> {
        let base = fs::canonicalize(std::env::temp_dir()).map_err(|error| {
            before_start(format!("Could not access process input directory: {error}"))
        })?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory = base.join(format!(
            "jecode-input-{}-{}-{stamp}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|error| {
                before_start(format!("Could not create process input directory: {error}"))
            })?;
        let path = directory.join("stdin");
        let mut pipe = Self {
            directory,
            path,
            reader: None,
            writer: None,
        };
        let status = Command::new("mkfifo")
            .args(["-m", "600"])
            .arg(pipe.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| before_start(format!("Could not run mkfifo: {error}")))?;
        if !status.success() {
            return Err(before_start(
                "Could not create private process input pipe".into(),
            ));
        }
        let open = |write: bool| {
            OpenOptions::new()
                .read(!write)
                .write(write)
                .custom_flags(NONBLOCK)
                .open(&pipe.path)
                .map_err(|error| {
                    before_start(format!("Could not open process input pipe: {error}"))
                })
        };
        pipe.reader = Some(open(false)?);
        pipe.writer = Some(open(true)?);
        Ok(pipe)
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn send(
        mut self,
        bytes: Vec<u8>,
        stop: Arc<AtomicBool>,
        cancellation: Cancellation,
    ) -> JoinHandle<io::Result<()>> {
        drop(self.reader.take());
        let mut writer = self.writer.take().unwrap();
        // The target has opened the FIFO. Unlink it before sending any input.
        self.unlink();
        thread::spawn(move || {
            let mut remaining = bytes.as_slice();
            while !remaining.is_empty()
                && !stop.load(Ordering::Acquire)
                && !cancellation.requested()
            {
                match writer.write(remaining) {
                    Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                    Ok(length) => remaining = &remaining[length..],
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        })
    }

    fn unlink(&self) {
        let _ = fs::remove_file(self.path());
        let _ = fs::remove_dir(&self.directory);
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        self.unlink();
    }
}
