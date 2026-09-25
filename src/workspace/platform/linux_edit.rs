//! Directory-relative, no-replace publication. Existing inodes are never truncated.
use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

unsafe extern "C" {
    fn renameat2(
        oldfd: i32,
        old: *const std::ffi::c_char,
        newfd: i32,
        new: *const std::ffi::c_char,
        flags: u32,
    ) -> i32;
    fn unlinkat(fd: i32, path: *const std::ffi::c_char, flags: i32) -> i32;
    fn flistxattr(fd: i32, list: *mut std::ffi::c_char, size: usize) -> isize;
}

pub fn identity(file: &File) -> io::Result<(u64, u64)> {
    let meta = file.metadata()?;
    Ok((meta.dev(), meta.ino()))
}
pub fn editable(file: &File) -> io::Result<()> {
    let meta = file.metadata()?;
    // This first text-edit path preserves ordinary Unix modes. Reject additional
    // security/metadata rather than silently dropping ACLs, labels or attributes.
    // SAFETY: null/zero requests the attribute-list size for a live descriptor.
    let attributes = unsafe { flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0) };
    if meta.nlink() != 1
        || meta.mode() & 0o7000 != 0
        || meta.permissions().readonly()
        || attributes != 0
    {
        return Err(io::ErrorKind::Unsupported.into());
    }
    Ok(())
}
pub fn edit_open(parent: &File, name: &str) -> io::Result<File> {
    open_file(parent, name, 0, 0, false)
}
pub fn create(parent: &File, name: &str) -> io::Result<File> {
    // O_RDWR | O_CREAT | O_EXCL; owner-only until the target mode is installed.
    open_file(parent, name, 2 | 0x40 | 0x80, 0o600, false)
}
pub fn metadata_to(source: &File, target: &File) -> io::Result<()> {
    let source_meta = source.metadata()?;
    let target_meta = target.metadata()?;
    if source_meta.uid() != target_meta.uid() || source_meta.gid() != target_meta.gid() {
        return Err(io::ErrorKind::Unsupported.into());
    }
    target.set_permissions(source.metadata()?.permissions())
}
pub fn capture_policy(source: &File) -> io::Result<Vec<u8>> {
    Ok((source.metadata()?.mode() & 0o777).to_le_bytes().to_vec())
}
pub fn apply_policy(target: &File, policy: &[u8]) -> io::Result<()> {
    let bytes: [u8; 4] = policy.try_into().map_err(|_| io::ErrorKind::InvalidData)?;
    let mode = u32::from_le_bytes(bytes);
    if mode & !0o777 != 0 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    target.set_permissions(std::fs::Permissions::from_mode(mode))
}
pub fn move_new(parent: &File, _: &File, from: &str, to: &str) -> io::Result<()> {
    let from = CString::new(from).map_err(|_| io::ErrorKind::InvalidInput)?;
    let to = CString::new(to).map_err(|_| io::ErrorKind::InvalidInput)?;
    // SAFETY: live directory descriptors, NUL-terminated single components.
    // RENAME_NOREPLACE never removes a competing file, symlink or directory.
    if unsafe {
        renameat2(
            parent.as_raw_fd(),
            from.as_ptr(),
            parent.as_raw_fd(),
            to.as_ptr(),
            1,
        )
    } != 0
    {
        let error = io::Error::last_os_error();
        return Err(if matches!(error.raw_os_error(), Some(22 | 95)) {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "filesystem does not support no-replace renames",
            )
        } else {
            error
        });
    }
    Ok(())
}
pub fn remove_owned(parent: &File, file: &File, name: &str) -> io::Result<()> {
    let entry = edit_open(parent, name)?;
    if identity(file)? != identity(&entry)? {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let name = CString::new(name).map_err(|_| io::ErrorKind::InvalidInput)?;
    // SAFETY: validated component below an owned directory. This only cleans an
    // unpublished staging file; recovery originals are never deleted here.
    if unsafe { unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
pub fn sync_parent(parent: &File) -> io::Result<()> {
    parent.sync_all()
}

#[cfg(test)]
#[test]
fn native_publication_operations() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/linux-edit-native");
    std::fs::create_dir_all(&base).unwrap();
    let dir = base.join(std::process::id().to_string());
    std::fs::create_dir(&dir).unwrap();
    let parent = root(&dir).unwrap();
    let result = (|| -> io::Result<()> {
        let stage = create(&parent, "stage")?;
        stage.sync_all()?;
        let moved = move_new(&parent, &stage, "stage", "published");
        if let Err(error) = &moved
            && base.starts_with("/mnt/c/")
            && error.kind() == io::ErrorKind::Unsupported
        {
            assert!(dir.join("stage").exists());
            assert!(!dir.join("published").exists());
            return Ok(());
        }
        moved?;
        Ok(())
    })();
    drop(parent);
    std::fs::remove_dir_all(&dir).unwrap();
    result.unwrap();
}
