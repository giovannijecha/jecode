//! Linux process group with nonblocking pipes. The unreaped leader reserves its
//! ID until group termination; deliberately detached sessions are not contained.
#![allow(unsafe_code)]
use crate::{
    command::{Channel, Exit, Shell},
    workspace::Directory,
};
use std::{
    fs::File,
    io::{self, Read},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::{CommandExt, ExitStatusExt},
    },
    process::{Child, Command, Stdio},
};

unsafe extern "C" {
    fn fchdir(fd: i32) -> i32;
    fn setsid() -> i32;
    fn fcntl(fd: i32, command: i32, ...) -> i32;
    fn kill(pid: i32, signal: i32) -> i32;
    fn waitid(kind: u32, id: u32, info: *mut u64, options: i32) -> i32;
    fn pipe2(fds: *mut i32, flags: i32) -> i32;
}
pub(in crate::command) struct Process {
    child: Child,
    stdout: File,
    stderr: File,
    finished: Option<Exit>,
}
impl Process {
    pub fn spawn(script: &str, directory: &Directory, _: &Shell) -> io::Result<Self> {
        let fd = directory.file().as_raw_fd();
        // Complete fallible pipe setup before spawning any executable code.
        let (stdout, stdout_write) = pipe()?;
        let (stderr, stderr_write) = pipe()?;
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", script])
            .current_dir(&directory.path)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout_write))
            .stderr(Stdio::from(stderr_write))
            .env("NO_COLOR", "1")
            .env("TERM", "dumb")
            .env("GIT_TERMINAL_PROMPT", "0");
        // SAFETY: only async-signal-safe syscalls and errno access after fork.
        // The held directory outlives spawn; CLOEXEC closes it after fchdir.
        unsafe {
            command.pre_exec(move || {
                if setsid() < 0 || fchdir(fd) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(Self {
            child: command.spawn()?,
            stdout,
            stderr,
            finished: None,
        })
    }
    pub fn read(&mut self, channel: Channel, buffer: &mut [u8]) -> io::Result<usize> {
        let reader = match channel {
            Channel::Stdout => &mut self.stdout,
            Channel::Stderr => &mut self.stderr,
        };
        match reader.read(buffer) {
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(0)
            }
            result => result,
        }
    }
    pub fn exited(&mut self) -> io::Result<bool> {
        // Linux siginfo_t is 128 bytes, aligned to 8 on our supported 64-bit ABIs.
        // Only si_signo (first int) is inspected; WNOWAIT retains the child's PID.
        let mut info = [0u64; 16];
        // SAFETY: full writable siginfo_t storage; P_PID, WNOHANG|WEXITED|WNOWAIT.
        let result = unsafe { waitid(1, self.child.id(), info.as_mut_ptr(), 1 | 4 | 0x01000000) };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(info[0] != 0)
    }
    pub fn finish(&mut self) -> io::Result<Exit> {
        if let Some(exit) = self.finished {
            return Ok(exit);
        }
        // SAFETY: group ID belongs to our unreaped child, never a recycled PID.
        if unsafe { kill(-(self.child.id() as i32), 9) } < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(3) {
                return Err(error);
            }
        }
        let status = self.child.wait()?;
        let exit = Exit {
            code: status.code().map(i64::from),
            signal: status.signal(),
        };
        self.finished = Some(exit);
        Ok(exit)
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn pipe() -> io::Result<(File, File)> {
    let mut fds = [-1; 2];
    // SAFETY: two writable fd slots; CLOEXEC is set atomically before other threads spawn.
    if unsafe { pipe2(fds.as_mut_ptr(), 0x80000) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: each successful new descriptor transfers to exactly one owner.
    let (read, write) = unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) };
    // Nonblocking applies only to our reader, never to the child's writer.
    let flags = unsafe { fcntl(read.as_raw_fd(), 3) };
    if flags < 0 || unsafe { fcntl(read.as_raw_fd(), 4, flags | 0x800) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((read, write))
}
