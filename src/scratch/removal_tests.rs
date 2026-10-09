use super::*;
use crate::test_support::Directory;

fn area() -> (Directory, Directory, Area) {
    let home = Directory::new();
    let project = Directory::new();
    let path = home.path().join("tmp").join("bucket").join("123-45-6");
    let area = Area::new(path, project.path().to_path_buf(), "123-45-6").unwrap();
    (home, project, area)
}

#[test]
fn missing_area_is_untouched_and_is_not_created() {
    let (_home, _project, area) = area();
    assert!(area.prepare_removal().unwrap().is_none());
    assert!(!area.path().exists());
}

#[test]
fn removal_includes_marker_root_and_orphaned_files() {
    let (_home, _project, area) = area();
    area.ensure().unwrap();
    let orphan = area.path().join("orphan").join("nested");
    fs::create_dir_all(&orphan).unwrap();
    fs::write(orphan.join("file.txt"), "owned").unwrap();
    let (files, dirs) = area.prepare_removal().unwrap().unwrap().remove().unwrap();
    assert_eq!((files, dirs), (2, 3));
    assert!(!area.path().exists());
}

#[test]
fn missing_or_foreign_marker_blocks_removal() {
    let (_home, _project, area) = area();
    fs::create_dir_all(area.path()).unwrap();
    let file = area.path().join("keep.txt");
    fs::write(&file, "keep").unwrap();
    assert!(area.prepare_removal().is_err());
    assert!(file.exists());
    fs::write(area.path().join(MARKER), "{}").unwrap();
    assert!(area.prepare_removal().is_err());
    assert!(file.exists());
}

#[cfg(unix)]
#[test]
fn linked_entry_blocks_removal_before_any_file_is_removed() {
    use std::os::unix::fs::symlink;
    let (_home, _project, area) = area();
    area.ensure().unwrap();
    let file = area.path().join("keep.txt");
    fs::write(&file, "keep").unwrap();
    symlink(&file, area.path().join("linked")).unwrap();
    assert!(area.prepare_removal().is_err());
    assert!(file.exists());
    assert!(area.path().join(MARKER).exists());
}

#[cfg(unix)]
#[test]
fn root_removal_failure_keeps_ownership_for_retry() {
    use std::os::unix::fs::PermissionsExt;
    let (_home, _project, area) = area();
    area.ensure().unwrap();
    fs::write(area.path().join("probe.txt"), "disposable").unwrap();
    let parent = area.path().parent().unwrap();
    let permissions = fs::metadata(parent).unwrap().permissions();
    fs::set_permissions(parent, fs::Permissions::from_mode(0o500)).unwrap();
    let result = area.prepare_removal().unwrap().unwrap().remove();
    let ownership = fs::read_to_string(area.path().join(MARKER)).ok();
    // Restore fixture access before assertions and teardown.
    fs::set_permissions(parent, permissions).unwrap();
    if result.is_ok() {
        return; // A privileged test process may bypass directory permissions.
    }
    assert_eq!(
        ownership.and_then(|text| json::parse(&text).ok()),
        Some(area.identity())
    );
    assert!(!area.path().join("probe.txt").exists());
    area.prepare_removal().unwrap().unwrap().remove().unwrap();
    assert!(!area.path().exists());
}

#[test]
fn a_late_file_keeps_the_owner_when_root_removal_fails() {
    let (_home, _project, area) = area();
    area.ensure().unwrap();
    let removal = area.prepare_removal().unwrap().unwrap();
    fs::write(area.path().join("late.txt"), "late").unwrap();
    let error = removal.remove().unwrap_err();
    assert!(
        error.contains("Temporary ownership was restored"),
        "{error}"
    );
    let (files, directories) = area.prepare_removal().unwrap().unwrap().remove().unwrap();
    assert_eq!((files, directories), (2, 1));
    assert!(!area.path().exists());
}
