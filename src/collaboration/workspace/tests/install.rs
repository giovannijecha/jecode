use super::{remove, stage};
use crate::test_support::Directory;
use std::fs;
use std::path::Path;

fn install(path: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    stage(path, bytes, mode)?.install()
}

#[cfg(windows)]
fn set_readonly(path: &Path, readonly: bool) {
    super::super::files::apply_mode(path, u32::from(readonly)).unwrap();
}

#[test]
fn dropping_staged_file_leaves_destination_untouched() {
    let directory = Directory::new();
    let path = directory.path().join("document.txt");
    fs::write(&path, b"original").unwrap();
    let pending = stage(&path, &b"replacement"[..], 0o644).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"original");
    drop(pending);
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[cfg(windows)]
#[test]
fn installs_readonly_file_with_exact_bytes() {
    let directory = Directory::new();
    let path = directory.path().join("document.txt");
    fs::write(&path, b"original").unwrap();
    set_readonly(&path, true);

    install(&path, b"replacement", 1).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"replacement");
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
    set_readonly(&path, false);
}

#[cfg(windows)]
#[test]
fn failed_install_restores_readonly_target() {
    use std::os::windows::fs::OpenOptionsExt;

    let directory = Directory::new();
    let path = directory.path().join("document.txt");
    fs::write(&path, b"original").unwrap();
    set_readonly(&path, true);
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0x1 | 0x2) // FILE_SHARE_READ | FILE_SHARE_WRITE; deny delete.
        .open(&path)
        .unwrap();

    assert!(install(&path, b"replacement", 0).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    drop(held);
    set_readonly(&path, false);
}

#[cfg(windows)]
#[test]
fn failed_remove_restores_readonly_target() {
    use std::os::windows::fs::OpenOptionsExt;

    let directory = Directory::new();
    let path = directory.path().join("document.txt");
    fs::write(&path, b"original").unwrap();
    set_readonly(&path, true);
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0x1 | 0x2) // FILE_SHARE_READ | FILE_SHARE_WRITE; deny delete.
        .open(&path)
        .unwrap();

    assert!(remove(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
    drop(held);
    set_readonly(&path, false);
}

#[cfg(windows)]
#[test]
fn dropping_readonly_staged_file_removes_it() {
    let directory = Directory::new();
    let path = directory.path().join("document.txt");
    let pending = stage(&path, &b"replacement"[..], 1).unwrap();
    drop(pending);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn replacing_then_removing_hardlink_does_not_change_alias_mode() {
    use std::os::unix::fs::PermissionsExt;

    let directory = Directory::new();
    let path = directory.path().join("document.txt");
    let alias = directory.path().join("alias.txt");
    fs::write(&path, b"original").unwrap();
    fs::hard_link(&path, &alias).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();

    install(&path, b"replacement", 0o640).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"replacement");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
        0o640
    );
    assert_eq!(fs::read(&alias).unwrap(), b"original");
    assert_eq!(
        fs::metadata(&alias).unwrap().permissions().mode() & 0o7777,
        0o400
    );

    remove(&path).unwrap();
    assert!(!path.exists());
    assert_eq!(fs::read(&alias).unwrap(), b"original");
    assert_eq!(
        fs::metadata(&alias).unwrap().permissions().mode() & 0o7777,
        0o400
    );
}
