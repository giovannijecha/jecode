//! Native permissions and no-follow opens. This is ordinary user-scoped storage.
#![allow(unsafe_code)]
use std::{
    fs::{self, DirBuilder, File, OpenOptions, Permissions},
    io,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};
unsafe extern "C" {
    fn geteuid() -> u32;
}

pub fn directory(path: &Path) -> io::Result<()> {
    DirBuilder::new().mode(0o700).create(path)
}
pub fn protect_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    // SAFETY: geteuid has no arguments or memory obligations.
    if metadata.uid() != unsafe { geteuid() } || !metadata.is_dir() {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    fs::set_permissions(path, Permissions::from_mode(0o700))?;
    if fs::metadata(path)?.mode() & 0o077 != 0 {
        return Err(io::ErrorKind::Unsupported.into());
    }
    Ok(())
}
pub fn read(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(0x20000 | 0x80000)
        .open(path)
}
pub fn create(path: &Path, existing: bool) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(existing)
        .create_new(!existing)
        .mode(0o600)
        .custom_flags(0x20000 | 0x80000)
        .open(path)
}
pub fn check_file(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    // SAFETY: geteuid has no arguments or memory obligations.
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(())
}
pub fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}
