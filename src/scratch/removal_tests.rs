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
        ownership.and_then(|text| crate::json::parse(&text).ok()),
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

#[test]
fn a_parent_replaced_after_preflight_does_not_redirect_removal() {
    let (home, _project, area) = area();
    area.ensure().unwrap();
    fs::write(area.path().join("probe.txt"), "owned").unwrap();
    let removal = area.prepare_removal().unwrap().unwrap();
    let outside = Directory::new();
    let outside_session = outside.path().join("123-45-6");
    fs::create_dir(&outside_session).unwrap();
    fs::write(outside_session.join(MARKER), area.identity().encode()).unwrap();
    let outside_file = outside_session.join("probe.txt");
    fs::write(&outside_file, "outside").unwrap();

    let parent = area.path().parent().unwrap();
    let original = parent.with_file_name("original-bucket");
    assert!(parent.starts_with(home.path()) && original.starts_with(home.path()));
    fs::rename(parent, &original).unwrap();
    #[cfg(windows)]
    {
        let created = std::process::Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(parent.to_string_lossy().replace('/', "\\"))
            .arg(outside.path().to_string_lossy().replace('/', "\\"))
            .output()
            .unwrap();
        assert!(created.status.success(), "{created:?}");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), parent).unwrap();
    let result = removal.remove();
    // Remove the replacement before restoring the original fixture directory.
    #[cfg(windows)]
    fs::remove_dir(parent).unwrap();
    #[cfg(unix)]
    fs::remove_file(parent).unwrap();
    fs::rename(&original, parent).unwrap();

    assert!(result.is_err(), "Removal followed a replaced parent");
    assert_eq!(fs::read_to_string(outside_file).unwrap(), "outside");
    assert_eq!(
        fs::read_to_string(area.path().join("probe.txt")).unwrap(),
        "owned"
    );
}

#[test]
fn a_changed_owner_blocks_prepared_cleanup_and_removal() {
    for clean_only in [false, true] {
        let (_home, _project, area) = area();
        let root = area.ensure().unwrap();
        let file = root.join("keep.txt");
        fs::write(&file, "keep").unwrap();
        let removal = area
            .owned_directory(root)
            .prepare_removal()
            .unwrap()
            .unwrap();
        fs::write(area.path().join(MARKER), "{}").unwrap();
        let result = if clean_only {
            removal.clean()
        } else {
            removal.remove().map(|_| ())
        };
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "keep");
    }
}

#[test]
fn a_linked_marker_blocks_prepared_cleanup_and_removal() {
    for clean_only in [false, true] {
        let (_home, _project, area) = area();
        let root = area.ensure().unwrap();
        let file = root.join("keep.txt");
        fs::write(&file, "keep").unwrap();
        let removal = area
            .owned_directory(root)
            .prepare_removal()
            .unwrap()
            .unwrap();
        let outside = Directory::new();
        let original = outside.path().join("owner.json");
        let marker = area.path().join(MARKER);
        fs::rename(&marker, &original).unwrap();
        #[cfg(windows)]
        if std::os::windows::fs::symlink_file(&original, &marker).is_err() {
            return; // Windows requires Developer Mode or the symlink privilege.
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&original, &marker).unwrap();
        let result = if clean_only {
            removal.clean()
        } else {
            removal.remove().map(|_| ())
        };
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "keep");
        assert_eq!(
            fs::read_to_string(original).unwrap(),
            area.identity().encode()
        );
    }
}
