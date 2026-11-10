//! Another agent's temporary files are found read-only, by ownership record,
//! and never through a link or junction.

use super::*;
use crate::{
    sessions::Store,
    test_support::{Directory, link_directory},
};

/// Writes `notes.md` in the area of `session`, saved in `base/child-home`.
fn written(base: &Path, project: &Directory, session: &str) -> PathBuf {
    let area = Store::new(base.join("child-home"), project.path())
        .unwrap()
        .temporary_area(session)
        .unwrap();
    let (_, file) = area.file("notes.md", true).unwrap();
    fs::write(&file, "notes").unwrap();
    file
}

#[test]
fn files_are_found_by_session_and_ownership_record_without_creating_areas() {
    let base = Directory::new();
    let project = Directory::new();
    let home = base.path().join("child-home");
    assert_eq!(find(base.path(), &home, "1-1-1", "notes.md").unwrap(), None);
    assert!(!base.path().join("child-home").exists());

    let file = written(base.path(), &project, "1-1-1");
    let (_, path) = find(base.path(), &home, "1-1-1", "notes.md")
        .unwrap()
        .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "notes");
    assert_eq!(path, fs::canonicalize(&file).unwrap());
    assert!(file.ends_with(std::path::Path::new("1-1-1").join("tmp").join("notes.md")));
    assert_eq!(find(base.path(), &home, "1-1-1", "other.md").unwrap(), None);
    assert_eq!(find(base.path(), &home, "2-2-2", "notes.md").unwrap(), None);

    let marker = file.parent().unwrap().join(MARKER);
    let canonical = fs::canonicalize(project.path()).unwrap();
    fs::write(&marker, OWNERSHIP.identity(&canonical, "3-3-3").encode()).unwrap();
    let error = find(base.path(), &home, "1-1-1", "notes.md").unwrap_err();
    assert!(error.contains("another folder or session"), "{error}");
}

#[test]
fn links_and_junctions_are_never_followed() {
    let base = Directory::new();
    let project = Directory::new();
    let outside = Directory::new();
    let home = base.path().join("child-home");

    // A linked folder of areas is skipped without being entered.
    written(outside.path(), &project, "1-1-1");
    let folder = fs::read_dir(outside.path().join("child-home/sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::create_dir_all(home.join("sessions")).unwrap();
    link_directory(&folder, &home.join("sessions").join("linked"));
    assert_eq!(find(base.path(), &home, "1-1-1", "notes.md").unwrap(), None);

    // A linked directory inside an area refuses the read.
    let file = written(base.path(), &project, "2-2-2");
    fs::write(outside.path().join("x.md"), "outside").unwrap();
    link_directory(outside.path(), &file.parent().unwrap().join("sub"));
    let error = find(base.path(), &home, "2-2-2", "sub/x.md").unwrap_err();
    assert!(error.contains("link or junction"), "{error}");

    // So does a linked session folder beside it.
    let bucket = file.ancestors().nth(3).unwrap();
    let foreign = written(outside.path(), &project, "3-3-3");
    link_directory(foreign.ancestors().nth(2).unwrap(), &bucket.join("3-3-3"));
    let error = find(base.path(), &home, "3-3-3", "notes.md").unwrap_err();
    assert!(error.contains("link or junction"), "{error}");

    // So does a linked home.
    let linked = base.path().join("linked-home");
    link_directory(&base.path().join("child-home"), &linked);
    let error = find(base.path(), &linked, "2-2-2", "notes.md").unwrap_err();
    assert!(error.contains("link or junction"), "{error}");
}
