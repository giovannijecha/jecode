use super::*;
use crate::checkpoints::frame::Encoding;
use crate::checkpoints::index::MAX_DEPTH;
use crate::checkpoints::record::State;
use crate::checkpoints::show::{self, Content};

fn latest_point(workspace: &Workspace, kind: &str) -> u64 {
    index(workspace)
        .points
        .values()
        .rev()
        .find(|point| point.kind == kind)
        .unwrap_or_else(|| panic!("no {kind} point"))
        .seq
}

#[test]
fn crlf_unicode_paths_renames_and_directories_round_trip_exactly() {
    let fixture = Fixture::new();
    let unicode = "docs/Ünïcødé 名前/notes ✓.txt";
    fixture.write(unicode, "first line\r\nsecond line\r\n".as_bytes());
    fixture.write("old name.txt", b"moved content\n");
    fs::create_dir_all(fixture.path("empty/nested")).unwrap();
    let workspace = fixture.open();
    let start = latest_point(&workspace, "baseline");
    assert!(index(&workspace).current("empty/nested").is_some());

    fixture.write(unicode, b"rewritten\n");
    fs::rename(fixture.path("old name.txt"), fixture.path("new name.txt")).unwrap();
    fs::remove_dir_all(fixture.path("empty")).unwrap();
    fs::create_dir_all(fixture.path("later/tree")).unwrap();
    fixture.write("later/tree/leaf.txt", b"leaf\n");
    workspace
        .mark("milestone", "reshaped", "test", false, false)
        .unwrap();

    // A rename is a deletion plus a creation; both sides restore.
    let index_now = index(&workspace);
    assert!(index_now.current_state("old name.txt") == State::Absent);
    assert!(index_now.current_state("new name.txt").exists());

    let report = workspace
        .restore(&request(&workspace, start, None, Expected::Latest))
        .unwrap();
    assert!(
        report.failed.is_none() && report.conflicts.is_empty(),
        "{:?}",
        report.failed
    );
    assert_eq!(
        fixture.read(unicode).unwrap(),
        b"first line\r\nsecond line\r\n"
    );
    assert_eq!(fixture.read("old name.txt").unwrap(), b"moved content\n");
    assert!(fixture.read("new name.txt").is_none());
    assert!(fixture.path("empty/nested").is_dir());
    assert!(!fixture.path("later").exists());

    // The historical text keeps its line endings when read back.
    let reference = format!("checkpoint:cp-{start}:{unicode}");
    match show::content(&workspace, &reference).unwrap() {
        Content::Text(text) => assert_eq!(text, "first line\r\nsecond line\r\n"),
        _ => panic!("expected text"),
    }
    match show::content(&workspace, &format!("checkpoint:cp-{start}:docs")).unwrap() {
        Content::Directory(entries) => assert_eq!(entries, ["Ünïcødé 名前/"]),
        _ => panic!("expected a directory"),
    }
}

#[test]
fn an_archive_inside_the_workspace_excludes_itself() {
    let root = Directory::new();
    let home = root.path().join("state-home");
    fs::write(root.path().join("app.txt"), b"app\n").unwrap();
    let workspace = workspace::open(&home, root.path(), Options::offline()).unwrap();
    workspace.initialize().unwrap();
    fs::write(root.path().join("app.txt"), b"changed\n").unwrap();
    workspace
        .mark("milestone", "after", "test", false, false)
        .unwrap();
    let index = index(&workspace);
    assert!(index.current("app.txt").is_some());
    assert!(home.join("checkpoints").is_dir());
    assert!(
        index
            .paths
            .keys()
            .all(|path| !path.starts_with("state-home")),
        "{:?}",
        index.paths.keys().collect::<Vec<_>>()
    );
}

#[test]
fn offline_changes_are_reconciled_when_the_workspace_reopens() {
    let fixture = Fixture::new();
    fixture.write("kept.txt", b"kept\n");
    fixture.write("changed.txt", b"before\n");
    fixture.write("deleted.txt", b"deleted\n");
    fixture.write("renamed.txt", b"renamed\n");
    let workspace = fixture.open();
    let start = latest_point(&workspace, "baseline");
    drop(workspace);

    fixture.write("changed.txt", b"after, while closed\n");
    fs::remove_file(fixture.path("deleted.txt")).unwrap();
    fs::create_dir_all(fixture.path("moved")).unwrap();
    fs::rename(
        fixture.path("renamed.txt"),
        fixture.path("moved/renamed.txt"),
    )
    .unwrap();
    fixture.write("added.txt", b"added\n");

    let reopened = fixture.open();
    let recorded = index(&reopened);
    let reconcile = recorded
        .points
        .values()
        .rev()
        .find(|point| point.kind == "reconcile")
        .expect("a reconcile point");
    assert!(reconcile.seq > start);
    assert_eq!(recorded.current("changed.txt").unwrap().origin, "external");
    assert_eq!(recorded.current_state("deleted.txt"), State::Absent);
    assert!(recorded.current_state("moved/renamed.txt").exists());
    assert!(recorded.current_state("added.txt").exists());

    // The pre-close state is still restorable after the restart.
    let report = reopened
        .restore(&request(&reopened, start, None, Expected::Latest))
        .unwrap();
    assert!(report.failed.is_none(), "{:?}", report.failed);
    assert_eq!(fixture.read("changed.txt").unwrap(), b"before\n");
    assert_eq!(fixture.read("deleted.txt").unwrap(), b"deleted\n");
    assert_eq!(fixture.read("renamed.txt").unwrap(), b"renamed\n");
    assert!(fixture.read("added.txt").is_none());
    assert!(!fixture.path("moved").exists());

    // An unchanged reopen records coverage but no new point.
    let points = index(&reopened).points.len();
    drop(reopened);
    let again = fixture.open();
    assert_eq!(index(&again).points.len(), points);
}

#[test]
fn edit_chains_stay_bounded_and_every_version_reads_back() {
    let fixture = Fixture::new();
    let mut lines: Vec<String> = (0..4000)
        .map(|line| format!("line {line} of the file\n"))
        .collect();
    fixture.write("long.txt", lines.concat().as_bytes());
    let workspace = fixture.open();
    let mut expected = vec![(latest_point(&workspace, "baseline"), lines.concat())];
    let edits = MAX_DEPTH as usize * 2 + 3;
    for edit in 0..edits {
        let line = edit * 97 % lines.len();
        lines[line] = format!("edited {edit}\n");
        fixture.write("long.txt", lines.concat().as_bytes());
        let point = workspace
            .mark("milestone", &format!("edit {edit}"), "test", false, false)
            .unwrap();
        expected.push((point, lines.concat()));
    }

    let index = index(&workspace);
    let objects: Vec<_> = index
        .paths
        .get("long.txt")
        .unwrap()
        .iter()
        .filter_map(|version| match version.state {
            State::File { object, .. } => index.objects.get(&object),
            _ => None,
        })
        .collect();
    assert_eq!(objects.len(), edits + 1);
    assert!(objects.iter().all(|object| object.depth <= MAX_DEPTH));
    let deltas = objects
        .iter()
        .filter(|object| matches!(object.meta.encoding, Encoding::Delta { .. }))
        .count();
    // Small edits are stored as deltas, and long chains restart from a full copy.
    assert!(deltas > edits / 2, "{deltas} deltas of {}", objects.len());
    assert!(objects.iter().skip(1).any(|object| object.depth == 0));
    let stored: u64 = objects.iter().map(|object| object.location.length).sum();
    assert!(
        stored < (lines.concat().len() * 8) as u64,
        "{stored} bytes stored"
    );

    for (point, text) in &expected {
        match show::content(&workspace, &format!("checkpoint:cp-{point}:long.txt")).unwrap() {
            Content::Text(found) => assert!(found == *text, "cp-{point} differs"),
            _ => panic!("expected text"),
        }
    }
    assert!(show::content(&workspace, "checkpoint:cp-1:../outside").is_err());
}

#[test]
fn a_restore_stopped_partway_reports_progress_and_can_be_undone() {
    let fixture = Fixture::new();
    let names = ["a.txt", "z.txt", "m/locked.txt", "m/n.txt"];
    for name in names {
        fixture.write(name, b"original\n");
    }
    let Some(mut blocker) = Blocker::new(&fixture) else {
        // A privileged Unix account can write into read-only directories.
        return;
    };
    let workspace = fixture.open();
    let start = latest_point(&workspace, "baseline");
    blocker.lift();
    for name in names {
        fixture.write(name, b"changed\n");
    }
    blocker.engage();
    workspace
        .mark("milestone", "changed", "test", false, false)
        .unwrap();

    let report = workspace
        .restore(&request(&workspace, start, None, Expected::Latest))
        .unwrap();
    blocker.lift();
    assert_eq!(restored(&report), ["a.txt", "z.txt"]);
    let (failed, _) = report.failed.clone().expect("a failed step");
    assert_eq!(failed, "m/locked.txt");
    assert_eq!(report.remaining, ["m/n.txt"]);
    assert_eq!(fixture.read("a.txt").unwrap(), b"original\n");
    assert_eq!(fixture.read("m/locked.txt").unwrap(), b"changed\n");
    assert_eq!(fixture.read("m/n.txt").unwrap(), b"changed\n");
    // The failed path is recorded as found, never as restored.
    match index(&workspace).current_state("m/locked.txt") {
        State::File { object, .. } => {
            assert_eq!(&workspace.store().bytes(object).unwrap()[..], b"changed\n");
        }
        State::Unreadable { .. } => {}
        other => panic!("{other:?}"),
    }

    // The state saved before the restore brings back the partial change.
    let undo = workspace
        .restore(&request(
            &workspace,
            report.before.unwrap(),
            None,
            Expected::Latest,
        ))
        .unwrap();
    assert!(
        undo.failed.is_none() && undo.conflicts.is_empty(),
        "{:?}",
        undo.failed
    );
    for name in names {
        assert_eq!(fixture.read(name).unwrap(), b"changed\n", "{name}");
    }
}

/// Makes `m/locked.txt` impossible to replace while engaged; dropping lifts it.
struct Blocker {
    directory: std::path::PathBuf,
    #[cfg(windows)]
    lock: Option<fs::File>,
}

impl Blocker {
    /// Engages the block, or returns `None` when this account bypasses it.
    fn new(fixture: &Fixture) -> Option<Self> {
        let mut blocker = Self {
            directory: fixture.path("m"),
            #[cfg(windows)]
            lock: None,
        };
        blocker.engage();
        blocker.holds().then_some(blocker)
    }
}

#[cfg(windows)]
impl Blocker {
    /// Others may still read the file, so planning sees no conflict; only
    /// the replacement during installation fails.
    fn engage(&mut self) {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        self.lock = fs::File::options()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(self.directory.join("locked.txt"))
            .ok();
    }

    fn lift(&mut self) {
        self.lock = None;
    }

    fn holds(&self) -> bool {
        self.lock.is_some()
    }
}

#[cfg(unix)]
impl Blocker {
    /// A read-only directory. It is engaged whenever the workspace is
    /// recorded, so a restore plan has no directory change that lifts it.
    fn engage(&mut self) {
        set_mode(&self.directory, 0o555);
    }

    fn lift(&mut self) {
        set_mode(&self.directory, 0o755);
    }

    fn holds(&self) -> bool {
        let probe = self.directory.join("probe");
        let writable = fs::write(&probe, b"").is_ok();
        let _ = fs::remove_file(&probe);
        !writable
    }
}

impl Drop for Blocker {
    fn drop(&mut self) {
        self.lift();
    }
}

#[test]
fn unreadable_files_are_recorded_without_blocking_the_rest() {
    let fixture = Fixture::new();
    fixture.write("open.txt", b"open\n");
    fixture.write("held.txt", b"held\n");
    let Some(holder) = hold(&fixture.path("held.txt")) else {
        return;
    };
    let workspace = fixture.open();
    let recorded = index(&workspace);
    assert!(recorded.current_state("open.txt").exists());
    assert!(
        matches!(recorded.current_state("held.txt"), State::Unreadable { .. }),
        "{:?}",
        recorded.current_state("held.txt")
    );
    let baseline = latest_point(&workspace, "baseline");
    drop(holder);

    // Nothing was saved for it, so a restore says so instead of deleting it.
    fixture.write("held.txt", b"changed\n");
    fixture.write("open.txt", b"changed\n");
    let selected = paths(&["held.txt", "open.txt"]);
    let report = workspace
        .restore(&request(
            &workspace,
            baseline,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert_eq!(restored(&report), ["open.txt"]);
    assert!(
        report
            .skipped
            .iter()
            .any(|(path, reason)| path == "held.txt" && reason.contains("not captured")),
        "{:?}",
        report.skipped
    );
    assert_eq!(fixture.read("held.txt").unwrap(), b"changed\n");
}

/// Denies reading a file until dropped.
struct Held {
    #[cfg(windows)]
    _lock: fs::File,
    #[cfg(unix)]
    path: std::path::PathBuf,
}

#[cfg(windows)]
fn hold(path: &std::path::Path) -> Option<Held> {
    use std::os::windows::fs::OpenOptionsExt;
    let lock = fs::File::options()
        .read(true)
        .share_mode(0)
        .open(path)
        .ok()?;
    Some(Held { _lock: lock })
}

/// Returns `None` when this account reads the file anyway.
#[cfg(unix)]
fn hold(path: &std::path::Path) -> Option<Held> {
    set_mode(path, 0o000);
    let held = Held {
        path: path.to_path_buf(),
    };
    fs::read(path).is_err().then_some(held)
}

#[cfg(unix)]
impl Drop for Held {
    fn drop(&mut self) {
        set_mode(&self.path, 0o644);
    }
}

#[cfg(unix)]
fn set_mode(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}
