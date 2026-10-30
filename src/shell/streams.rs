//! Standard streams of a running command: the process's own, files, pipes
//! between pipeline stages, or the discarding null device.
use std::fs::File;
use std::io::{self, PipeReader, PipeWriter, Read, Write};
use std::process::Stdio;

pub(super) enum Source {
    Stdin,
    Null,
    File(File),
    Pipe(PipeReader),
}

pub(super) enum Sink {
    Stdout,
    Stderr,
    Null,
    File(File),
    Pipe(PipeWriter),
}

pub(super) struct Io {
    pub stdin: Source,
    pub stdout: Sink,
    pub stderr: Sink,
}

impl Io {
    pub fn standard() -> Self {
        Self {
            stdin: Source::Stdin,
            stdout: Sink::Stdout,
            stderr: Sink::Stderr,
        }
    }

    pub fn try_clone(&self) -> Result<Self, String> {
        let clone = || -> io::Result<Self> {
            Ok(Self {
                stdin: self.stdin.try_clone()?,
                stdout: self.stdout.try_clone()?,
                stderr: self.stderr.try_clone()?,
            })
        };
        clone().map_err(|error| error.to_string())
    }

    /// Writes a shell diagnostic to this command's error stream.
    pub fn report(&self, message: &str) {
        let _ = self
            .stderr
            .write_all(format!("shell: {message}\n").as_bytes());
    }
}

impl Source {
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(match self {
            Self::Stdin => Self::Stdin,
            Self::Null => Self::Null,
            Self::File(file) => Self::File(file.try_clone()?),
            Self::Pipe(pipe) => Self::Pipe(pipe.try_clone()?),
        })
    }

    pub fn stdio(&self) -> io::Result<Stdio> {
        Ok(match self {
            Self::Stdin => Stdio::inherit(),
            Self::Null => Stdio::null(),
            Self::File(file) => file.try_clone()?.into(),
            Self::Pipe(pipe) => pipe.try_clone()?.into(),
        })
    }
}

impl Read for &Source {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Source::Stdin => io::stdin().read(buffer),
            Source::Null => Ok(0),
            Source::File(file) => (&mut &*file).read(buffer),
            Source::Pipe(pipe) => (&mut &*pipe).read(buffer),
        }
    }
}

impl Sink {
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(match self {
            Self::Stdout => Self::Stdout,
            Self::Stderr => Self::Stderr,
            Self::Null => Self::Null,
            Self::File(file) => Self::File(file.try_clone()?),
            Self::Pipe(pipe) => Self::Pipe(pipe.try_clone()?),
        })
    }

    /// The child-process handle for this sink when used as `stream` 1 or 2.
    pub fn stdio(&self, stream: u8) -> io::Result<Stdio> {
        Ok(match self {
            Self::Stdout if stream == 1 => Stdio::inherit(),
            Self::Stderr if stream == 2 => Stdio::inherit(),
            Self::Stdout => io::stdout().into(),
            Self::Stderr => io::stderr().into(),
            Self::Null => Stdio::null(),
            Self::File(file) => file.try_clone()?.into(),
            Self::Pipe(pipe) => pipe.try_clone()?.into(),
        })
    }

    pub fn write_all(&self, data: &[u8]) -> io::Result<()> {
        let mut writer = self;
        Write::write_all(&mut writer, data)?;
        Write::flush(&mut writer)
    }
}

impl Write for &Sink {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        match self {
            Sink::Stdout => io::stdout().write(data),
            Sink::Stderr => io::stderr().write(data),
            Sink::Null => Ok(data.len()),
            Sink::File(file) => (&mut &*file).write(data),
            Sink::Pipe(pipe) => (&mut &*pipe).write(data),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Sink::Stdout => io::stdout().flush(),
            Sink::Stderr => io::stderr().flush(),
            Sink::Null | Sink::File(_) | Sink::Pipe(_) => Ok(()),
        }
    }
}

/// A closed downstream pipe ends a producer quietly, as SIGPIPE would.
pub(super) fn closed(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::BrokenPipe
}
