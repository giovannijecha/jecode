use super::*;
use crate::test_support::Directory;

#[cfg(unix)]
#[test]
fn instruction_symlinks_must_resolve_inside_the_launch_directory() {
    use std::os::unix::fs::symlink;

    let directory = Directory::new();
    let outside = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    let target = root.join("RULES.md");
    fs::write(&target, "Linked project rules").unwrap();
    let path = root.join(FILE_NAME);
    symlink(&target, &path).unwrap();
    assert!(load(&root).unwrap().contains("Linked project rules"));
    fs::remove_file(&path).unwrap();
    let target = outside.path().join("RULES.md");
    fs::write(&target, "Outside project rules").unwrap();
    symlink(fs::canonicalize(target).unwrap(), &path).unwrap();
    assert!(
        load(&root)
            .unwrap_err()
            .contains("inside the working directory")
    );
    fs::remove_file(&path).unwrap();
    symlink(root.join("MISSING.md"), path).unwrap();
    assert!(load(&root).unwrap_err().contains("Could not resolve"));
}

#[cfg(unix)]
#[test]
fn directory_symlinks_are_not_traversed_even_when_they_point_inside_root() {
    use std::os::unix::fs::symlink;

    let directory = Directory::new();
    let outside = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    fs::create_dir(root.join("real")).unwrap();
    fs::write(root.join("real/AGENTS.md"), "Real directory rules").unwrap();
    fs::write(outside.path().join(FILE_NAME), "Outside rules").unwrap();
    symlink(root.join("real"), root.join("linked-inside")).unwrap();
    symlink(outside.path(), root.join("linked-outside")).unwrap();
    let loaded = load(&root).unwrap();
    assert_eq!(loaded.matches("Real directory rules").count(), 1);
    assert!(!loaded.contains("Outside rules"));
    assert!(!loaded.contains("linked-inside"));
}

#[cfg(windows)]
#[test]
fn junctions_are_not_traversed_even_when_they_point_inside_root() {
    let directory = Directory::new();
    let outside = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    fs::create_dir(root.join("real")).unwrap();
    fs::write(root.join("real/AGENTS.md"), "Real directory rules").unwrap();
    fs::write(outside.path().join(FILE_NAME), "Outside rules").unwrap();
    for (name, target) in [
        ("linked-inside", root.join("real")),
        ("linked-outside", outside.path().to_path_buf()),
    ] {
        let link = root.join(name);
        let created = std::process::Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(target.to_string_lossy().replace('/', "\\"))
            .output()
            .unwrap();
        assert!(created.status.success(), "{created:?}");
    }
    let loaded = load(&root).unwrap();
    assert_eq!(loaded.matches("Real directory rules").count(), 1);
    assert!(!loaded.contains("Outside rules"));
    assert!(!loaded.contains("linked-inside"));
}

#[cfg(windows)]
#[test]
fn file_symlinks_resolve_only_inside_root_when_available() {
    use std::os::windows::fs::symlink_file;

    let directory = Directory::new();
    let outside = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    fs::write(root.join("rules.txt"), "Linked file inside root").unwrap();
    let path = root.join(FILE_NAME);
    if symlink_file(root.join("rules.txt"), &path).is_err() {
        return; // Creating symlinks requires Windows Developer Mode or a privilege.
    }
    assert!(load(&root).unwrap().contains("Linked file inside root"));
    fs::remove_file(&path).unwrap();
    fs::write(outside.path().join(FILE_NAME), "Outside rules").unwrap();
    symlink_file(outside.path().join(FILE_NAME), &path).unwrap();
    assert!(
        load(&root)
            .unwrap_err()
            .contains("inside the working directory")
    );
}
