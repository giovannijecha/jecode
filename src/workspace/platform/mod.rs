#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(super) use windows::*;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod linux;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub(super) use linux::*;

#[cfg(not(any(
    windows,
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
)))]
mod unsupported {
    use super::super::Opened;
    use std::{ffi::OsString, fs::File, io, path::Path};
    pub fn root(_: &Path) -> io::Result<File> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn open(_: &File, _: &str, _: bool) -> io::Result<Opened> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn absolute(_: &Path, _: bool) -> io::Result<Opened> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn names(_: &File, _: &mut dyn FnMut(OsString) -> bool) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn identity(_: &File) -> io::Result<(u64, u64)> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn editable(_: &File) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn edit_open(_: &File, _: &str) -> io::Result<File> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn create(_: &File, _: &str) -> io::Result<File> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn metadata_to(_: &File, _: &File) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn move_new(_: &File, _: &File, _: &str, _: &str) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn remove_owned(_: &File, _: &File, _: &str) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}
#[cfg(not(any(
    windows,
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
)))]
pub(super) use unsupported::*;
