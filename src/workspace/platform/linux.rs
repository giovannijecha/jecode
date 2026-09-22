//! Linux 5.6+ openat2: beneath an owned directory, no symlinks or mount crossings.
#![allow(unsafe_code)]
use super::super::Opened;
use std::{
    ffi::{CString, OsString},
    fs::{File, OpenOptions},
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::OpenOptionsExt,
    },
    path::Path,
};
#[path = "linux_edit.rs"]
mod edit;
pub use edit::*;
#[repr(C)]
struct How {
    flags: u64,
    mode: u64,
    resolve: u64,
}
unsafe extern "C" {
    fn syscall(number: std::ffi::c_long, ...) -> std::ffi::c_long;
}
pub fn root(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(0x10000 | 0x20000 | 0x800)
        .open(path)
}
pub fn open(root: &File, path: &str, directory: bool) -> io::Result<Opened> {
    let file = open_file(root, path, 0, 0, directory)?;
    Ok(Opened {
        file,
        _parents: Vec::new(),
    })
}
fn open_file(root: &File, path: &str, flags: u64, mode: u64, directory: bool) -> io::Result<File> {
    let name = CString::new(path).map_err(|_| io::ErrorKind::InvalidInput)?;
    let how = How {
        // O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK, plus O_DIRECTORY for listings.
        flags: flags | 0x80000 | 0x20000 | 0x800 | if directory { 0x10000 } else { 0 },
        mode,
        resolve: 0x08 | 0x04 | 0x01, // BENEATH | NO_SYMLINKS | NO_XDEV.
    };
    // SAFETY: immutable NUL-terminated path, valid root fd and initialized ABI struct.
    // 437 is openat2 on the supported Linux x86_64/aarch64 syscall ABIs.
    let fd = unsafe { syscall(437, root.as_raw_fd(), name.as_ptr(), &how, size_of::<How>()) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful syscall transfers a new owned file descriptor.
    let file = unsafe { File::from_raw_fd(fd as i32) };
    let kind = file.metadata()?.file_type();
    if if directory {
        !kind.is_dir()
    } else {
        !kind.is_file()
    } {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(file)
}
pub fn names(file: &File, visit: &mut dyn FnMut(OsString) -> bool) -> io::Result<()> {
    // This kernel-owned fd link refers to our held directory, not a model path.
    for entry in std::fs::read_dir(format!("/proc/self/fd/{}", file.as_raw_fd()))? {
        if !visit(entry?.file_name()) {
            break;
        }
    }
    Ok(())
}
