use super::{Mode, install, remove, stage, stage_link};
use crate::test_support::Directory;
use std::fs;

#[cfg(unix)]
struct ResetDirectoryMode(std::path::PathBuf);

#[cfg(unix)]
impl Drop for ResetDirectoryMode {
    fn drop(&mut self) {
        let _ = Mode(0o700).apply_path(&self.0);
    }
}

#[test]
fn dropping_staged_file_leaves_destination_untouched() {
    let directory = Directory::new();
    let path = directory.path().join("document.txt");
    fs::write(&path, b"original").unwrap();
    let pending = stage(&path, b"replacement", Mode::default()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"original");
    drop(pending);
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn failed_link_staging_leaves_live_file_untouched() {
    let directory = Directory::new();
    let path = directory.path().join("live");
    fs::write(&path, b"original").unwrap();
    assert!(stage_link("invalid\0target", false, &path).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[cfg(windows)]
#[test]
fn installs_readonly_file_with_exact_bytes() {
    let directory = Directory::new();
    let path = directory.path().join("document.txt");
    fs::write(&path, b"original").unwrap();
    Mode(Mode::READONLY).apply_path(&path).unwrap();

    install(&path, b"replacement", Mode(Mode::READONLY)).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"replacement");
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
    Mode::default().apply_path(&path).unwrap();
}

#[cfg(windows)]
#[test]
fn failed_install_restores_readonly_target() {
    use std::os::windows::fs::OpenOptionsExt;

    let directory = Directory::new();
    let path = directory.path().join("document.txt");
    fs::write(&path, b"original").unwrap();
    Mode(Mode::READONLY).apply_path(&path).unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0x1 | 0x2) // FILE_SHARE_READ | FILE_SHARE_WRITE; deny delete.
        .open(&path)
        .unwrap();

    assert!(install(&path, b"replacement", Mode::default()).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    drop(held);
    Mode::default().apply_path(&path).unwrap();
}

#[cfg(windows)]
#[test]
fn failed_remove_restores_readonly_target() {
    use std::os::windows::fs::OpenOptionsExt;

    let directory = Directory::new();
    let path = directory.path().join("document.txt");
    fs::write(&path, b"original").unwrap();
    Mode(Mode::READONLY).apply_path(&path).unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0x1 | 0x2) // FILE_SHARE_READ | FILE_SHARE_WRITE; deny delete.
        .open(&path)
        .unwrap();

    assert!(remove(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
    drop(held);
    Mode::default().apply_path(&path).unwrap();
}

#[cfg(windows)]
#[test]
fn dropping_readonly_staged_file_removes_it() {
    let directory = Directory::new();
    let path = directory.path().join("document.txt");
    let pending = stage(&path, b"replacement", Mode(Mode::READONLY)).unwrap();
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
    Mode(0o400).apply_path(&path).unwrap();

    install(&path, b"replacement", Mode(0o640)).unwrap();
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

#[cfg(unix)]
#[test]
fn directory_mode_can_restore_access_to_unreadable_directory() {
    use super::directory_mode;
    use std::os::unix::fs::PermissionsExt;

    let fixture = Directory::new();
    let path = fixture.path().join("locked");
    fs::create_dir(&path).unwrap();
    let _reset = ResetDirectoryMode(path.clone());
    Mode(0o000).apply_path(&path).unwrap();

    directory_mode(&path, Mode(0o700)).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
        0o700
    );
    fs::write(path.join("child"), b"accessible").unwrap();
}

#[cfg(unix)]
#[test]
fn directory_mode_can_sync_after_removing_all_access() {
    use super::directory_mode;
    use std::os::unix::fs::PermissionsExt;

    let fixture = Directory::new();
    let path = fixture.path().join("locked");
    fs::create_dir(&path).unwrap();
    let _reset = ResetDirectoryMode(path.clone());

    directory_mode(&path, Mode(0o000)).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
        0o000
    );
}
