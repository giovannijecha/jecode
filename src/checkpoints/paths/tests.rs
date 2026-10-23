use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "jecode-path-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn explicit_generated_file_and_unicode_missing_name_are_preserved() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("target")).unwrap();
    fs::write(fixture.0.join("target/owned.txt"), b"saved").unwrap();
    let chosen = discover(
        &fixture.0,
        &["target/owned.txt".into(), "é̀-missing.txt".into()],
        &[],
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(chosen.paths, ["target/owned.txt", "é̀-missing.txt"]);
    let directory = discover(
        &fixture.0,
        &["target".into()],
        &[],
        &Cancellation::default(),
    )
    .unwrap();
    assert!(directory.paths.contains(&"target/owned.txt".into()));
    assert_eq!(directory.roots, ["target"]);
    let mixed = discover(
        &fixture.0,
        &["../outside".into(), "target/owned.txt".into()],
        &[],
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(mixed.paths, ["target/owned.txt"]);
    assert_eq!(mixed.partial.len(), 1);
    let full = discover(&fixture.0, &[String::new()], &[], &Cancellation::default()).unwrap();
    assert!(!full.paths.iter().any(|path| path.starts_with("target/")));
    assert!(full.roots.contains(&String::new()));
}

#[test]
fn trailing_directory_separator_selects_a_complete_scope_and_exact_entry() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("generated")).unwrap();
    fs::write(fixture.0.join("generated/result.txt"), b"before").unwrap();
    let chosen = discover(
        &fixture.0,
        &["../outside".into(), "generated/".into()],
        &[],
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(chosen.partial.len(), 1);
    assert_eq!(chosen.roots, ["generated"]);
    assert_eq!(chosen.paths, ["generated", "generated/result.txt"]);
    let exact = discover_exact(
        &fixture.0,
        &["generated/".into()],
        &[],
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(exact.paths, ["generated"]);
    assert!(exact.roots.is_empty());
    let snapshot = Snapshot {
        reference: "test".into(),
        created: 0,
        tool: "write".into(),
        origin: "test".into(),
        entries: [("generated".into(), super::super::state::State::Absent)]
            .into_iter()
            .collect(),
        roots: Vec::new(),
        excluded: Vec::new(),
        ignored: Vec::new(),
        ignore_rules: IgnoreRules::default(),
        partial: Vec::new(),
    };
    assert_eq!(
        resolve_saved(&fixture.0, &snapshot, "generated/").unwrap(),
        "generated"
    );
    assert!(resolve_saved(&fixture.0, &snapshot, "../outside").is_err());
}

#[cfg(windows)]
#[test]
fn native_windows_separator_selects_directory() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("generated")).unwrap();
    fs::write(fixture.0.join("generated/result.txt"), b"before").unwrap();
    let chosen = discover(
        &fixture.0,
        &["generated\\".into()],
        &[],
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(chosen.roots, ["generated"]);
    assert_eq!(chosen.paths, ["generated", "generated/result.txt"]);
}

#[test]
fn directory_link_is_an_entry_and_never_a_scope_root() {
    let fixture = Fixture::new();
    let outside = Fixture::new();
    fs::write(outside.0.join("hidden"), b"outside").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside.0, fixture.0.join("link")).unwrap();
    #[cfg(windows)]
    if std::os::windows::fs::symlink_dir(&outside.0, fixture.0.join("link")).is_err() {
        return;
    }
    let found = discover(&fixture.0, &[String::new()], &[], &Cancellation::default()).unwrap();
    assert!(found.paths.contains(&"link".to_owned()));
    assert!(!found.paths.iter().any(|path| path == "link/hidden"));
    assert!(relative(&fixture.0, &fixture.0.join("link")).is_some());
    assert!(relative(&fixture.0, &outside.0.join("hidden")).is_none());
    let explicit = discover(
        &fixture.0,
        &["link/hidden".into()],
        &[],
        &Cancellation::default(),
    )
    .unwrap();
    assert!(explicit.paths.is_empty());
    assert!(!explicit.partial.is_empty());
}

#[cfg(windows)]
#[test]
fn case_aliases_keep_the_existing_final_entry_spelling() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("Folder")).unwrap();
    fs::write(fixture.0.join("Folder/Name.txt"), b"x").unwrap();
    assert_eq!(
        relative(&fixture.0, &fixture.0.join("folder/name.TXT")),
        Some("Folder/Name.txt".into())
    );
    let found = discover(
        &fixture.0,
        &["folder/name.TXT".into()],
        &[],
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(found.paths, ["Folder/Name.txt"]);
}
