#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(super) use windows::*;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(super) use linux::*;

#[cfg(not(any(windows, target_os = "linux")))]
mod unsupported {
    use std::{fs::File, io, path::Path};
    pub fn read(_: &Path) -> io::Result<File> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn create(_: &Path, _: bool) -> io::Result<File> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn directory(_: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn protect_directory(_: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn check_file(_: &File) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn sync_directory(_: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}
#[cfg(not(any(windows, target_os = "linux")))]
pub(super) use unsupported::*;
