//! An external writer changes a leased path after restore planning. Real
//! child-process stages make the timing deterministic without sleep races.
use super::*;
use crate::checkpoints::actions::{self, Caller};
use crate::checkpoints::record::State;
use crate::json::{self, Value};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const TARGET: &str = "JECODE_CHECKPOINT_TEST_TARGET";
const SELECTED: &str = "JECODE_CHECKPOINT_RACE_PATH";
const REPORT: &str = "RESTORE_REPORT:";

pub(super) fn child(home: &Path, root: &Path) {
    let workspace = workspace::open(home, root, Options::offline()).unwrap();
    workspace.initialize().unwrap();
    let mut fields = vec![("ref", Value::string(std::env::var(TARGET).unwrap()))];
    if let Ok(path) = std::env::var(SELECTED) {
        fields.push(("action", Value::string("restore")));
        fields.push(("paths", Value::Array(vec![Value::string(path)])));
    } else {
        fields.push(("action", Value::string("restore_all")));
    }
    let result = actions::run(
        &workspace,
        &Caller {
            origin: "test",
            user: true,
        },
        &crate::checkpoints::session::object(fields),
    )
    .unwrap();
    println!("{REPORT}{}", result.encode());
}

struct Running(Option<Child>);

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn run(
    fixture: &Fixture,
    target: u64,
    path: Option<&str>,
    stage: &str,
    change: impl FnOnce(),
) -> Value {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "checkpoints::tests::process::child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("JECODE_CHECKPOINT_CHILD", "restore-race")
        .env("JECODE_CHECKPOINT_CHILD_HOME", fixture.home.path())
        .env("JECODE_CHECKPOINT_CHILD_ROOT", fixture.root.path())
        .env(TARGET, crate::checkpoints::index::reference(target))
        .env("JECODE_CHECKPOINT_TEST_STAGE", stage)
        .env_remove(SELECTED)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = path {
        command.env(SELECTED, path);
    }
    let mut running = Running(Some(command.spawn().unwrap()));
    let ready = fixture
        .home
        .path()
        .join(format!("checkpoint-{stage}.ready"));
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready.exists() {
        assert!(
            running.0.as_mut().unwrap().try_wait().unwrap().is_none(),
            "child exited before {stage}"
        );
        assert!(Instant::now() < deadline, "restore did not reach {stage}");
        std::thread::sleep(Duration::from_millis(10));
    }
    change();
    fs::write(
        fixture
            .home
            .path()
            .join(format!("checkpoint-{stage}.release")),
        b"go",
    )
    .unwrap();
    while running.0.as_mut().unwrap().try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "restore did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = running.0.take().unwrap().wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    json::parse(
        text.lines()
            .find_map(|line| line.split_once(REPORT).map(|(_, report)| report))
            .unwrap(),
    )
    .unwrap()
}

fn baseline(fixture: &Fixture) -> u64 {
    index(&fixture.open())
        .points
        .values()
        .find(|point| point.kind == "baseline")
        .unwrap()
        .seq
}

fn conflict(fixture: &Fixture, result: &Value, path: &str, bytes: &[u8]) {
    assert!(result.get("failed").is_none(), "{result:?}");
    let conflicts = result.get("conflicts").unwrap().as_array().unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].get("path").unwrap().as_str(), Some(path));
    assert_eq!(fixture.read(path).unwrap(), bytes);
    let workspace = fixture.open();
    let history = index(&workspace);
    assert!(history.pending.is_empty());
    let version = history.current(path).unwrap();
    assert_eq!(version.origin, "external");
    let State::File { object, .. } = version.state else {
        panic!("external bytes were not saved")
    };
    assert_eq!(&workspace.store().bytes(object).unwrap()[..], bytes);
}

#[test]
fn external_write_during_full_restore_is_saved_and_stops_remaining_paths() {
    let fixture = Fixture::new();
    for path in ["a.txt", "b.txt", "c.txt"] {
        fixture.write(path, b"original\n");
    }
    let target = baseline(&fixture);
    for path in ["a.txt", "b.txt", "c.txt"] {
        fixture.write(path, b"before restore\n");
    }
    let result = run(&fixture, target, None, "restore-first-installed", || {
        assert_eq!(fixture.read("a.txt").unwrap(), b"original\n");
        fixture.write("b.txt", b"external editor\n");
    });
    conflict(&fixture, &result, "b.txt", b"external editor\n");
    assert_eq!(result.get("restored").unwrap().as_array().unwrap().len(), 1);
    assert_eq!(
        result.get("not_restored"),
        Some(&Value::Array(vec![Value::string("c.txt")]))
    );
    assert_eq!(fixture.read("c.txt").unwrap(), b"before restore\n");
    assert!(
        result
            .get("retry")
            .unwrap()
            .get("note")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("stopped after applying")
    );

    // Undo the installed file without replacing the concurrent editor's state.
    let workspace = fixture.open();
    let undo = result.get("undo").unwrap();
    assert_eq!(undo.get("exact"), Some(&Value::Bool(true)));
    assert_eq!(
        undo.get("paths"),
        Some(&Value::Array(vec![Value::string("a.txt")]))
    );
    let undone = actions::run(
        &workspace,
        &Caller {
            origin: "test",
            user: true,
        },
        undo,
    )
    .unwrap();
    assert!(undone.get("conflicts").is_none(), "{undone:?}");
    assert_eq!(fixture.read("a.txt").unwrap(), b"before restore\n");
    assert_eq!(fixture.read("b.txt").unwrap(), b"external editor\n");
}

#[test]
fn a_file_created_after_staging_is_preserved() {
    let fixture = Fixture::new();
    fixture.write("b.txt", b"original\n");
    let target = baseline(&fixture);
    fs::remove_file(fixture.path("b.txt")).unwrap();
    let result = run(
        &fixture,
        target,
        Some("b.txt"),
        "restore-before-install",
        || {
            fixture.write("b.txt", b"created by editor\n");
        },
    );
    conflict(&fixture, &result, "b.txt", b"created by editor\n");
    assert!(
        result
            .get("restored")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(fs::read_dir(fixture.root.path()).unwrap().count(), 1);
}

#[test]
fn a_file_changed_after_staging_is_preserved() {
    let fixture = Fixture::new();
    fixture.write("b.txt", b"original\n");
    let target = baseline(&fixture);
    fixture.write("b.txt", b"before restore\n");
    let result = run(
        &fixture,
        target,
        Some("b.txt"),
        "restore-before-install",
        || {
            fixture.write("b.txt", b"changed by editor\n");
        },
    );
    conflict(&fixture, &result, "b.txt", b"changed by editor\n");
    assert!(
        result
            .get("restored")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(fs::read_dir(fixture.root.path()).unwrap().count(), 1);
}

#[test]
fn a_file_changed_before_deletion_is_preserved() {
    let fixture = Fixture::new();
    let target = baseline(&fixture);
    fixture.write("b.txt", b"created after checkpoint\n");
    let result = run(
        &fixture,
        target,
        Some("b.txt"),
        "restore-before-install",
        || {
            fixture.write("b.txt", b"edited before deletion\n");
        },
    );
    conflict(&fixture, &result, "b.txt", b"edited before deletion\n");
    assert!(
        result
            .get("restored")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[cfg(windows)]
#[test]
fn failed_replacement_does_not_claim_an_external_edit_for_undo() {
    use crate::checkpoints::record::Outcome;
    use std::os::windows::fs::OpenOptionsExt;
    for external in [
        &b"external during failed replacement\n"[..],
        &b"original\n"[..],
    ] {
        let fixture = Fixture::new();
        fixture.write("b.txt", b"original\n");
        let target = baseline(&fixture);
        fixture.write("b.txt", b"before restore\n");
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2)
            .open(fixture.path("b.txt"))
            .unwrap();
        let result = run(
            &fixture,
            target,
            Some("b.txt"),
            "restore-after-check",
            || fixture.write("b.txt", external),
        );
        drop(held);

        assert_eq!(
            result.get("failed").unwrap().get("path").unwrap().as_str(),
            Some("b.txt")
        );
        assert!(
            result
                .get("restored")
                .unwrap()
                .as_array()
                .unwrap()
                .is_empty(),
            "{result:?}"
        );
        assert!(result.get("undo").is_none(), "{result:?}");
        assert_eq!(&fixture.read("b.txt").unwrap()[..], external);
        let history = index(&fixture.open());
        assert_eq!(history.current("b.txt").unwrap().origin, "external");
        let before = crate::checkpoints::index::parse_reference(
            result.get("before").unwrap().as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(history.outcomes.get(&(before + 1)), Some(&Outcome::Unknown));
    }
}

#[cfg(unix)]
#[test]
fn undo_of_an_exact_directory_restore_keeps_later_descendant_edits() {
    use crate::checkpoints::files::Mode;
    let fixture = Fixture::new();
    fixture.write("directory/file.txt", b"original\n");
    Mode(0o700).apply_path(&fixture.path("directory")).unwrap();
    let workspace = fixture.open();
    let target = index(&workspace).latest();
    Mode(0o500).apply_path(&fixture.path("directory")).unwrap();
    let caller = Caller {
        origin: "test",
        user: true,
    };
    let restored = actions::run(
        &workspace,
        &caller,
        &Value::object([
            ("action", Value::string("restore")),
            (
                "ref",
                Value::string(crate::checkpoints::index::reference(target)),
            ),
            ("paths", Value::Array(vec![Value::string("directory")])),
            ("exact", Value::Bool(true)),
        ]),
    )
    .unwrap();
    let undo = restored.get("undo").unwrap();
    assert_eq!(undo.get("exact"), Some(&Value::Bool(true)));
    fixture.write("directory/file.txt", b"later editor change\n");
    let undone = actions::run(&workspace, &caller, undo).unwrap();
    // Return traversal access before assertions and fixture cleanup.
    Mode(0o700).apply_path(&fixture.path("directory")).unwrap();
    assert!(undone.get("failed").is_none(), "{undone:?}");
    assert_eq!(
        fixture.read("directory/file.txt").unwrap(),
        b"later editor change\n"
    );
}

#[cfg(unix)]
#[test]
fn directory_sync_failure_keeps_only_unsynced_operations_recoverable() {
    use std::os::unix::fs::PermissionsExt;
    struct Reset(PathBuf, fs::Permissions);
    impl Drop for Reset {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, self.1.clone());
        }
    }
    let fixture = Fixture::new();
    let names = ["a/item.txt", "b/item.txt", "c/item.txt"];
    for path in names {
        fixture.write(path, b"original\n");
    }
    let target = baseline(&fixture);
    for path in names {
        fixture.write(path, b"before restore\n");
    }
    let parent = fixture.path("a");
    let reset = Reset(parent.clone(), fs::metadata(&parent).unwrap().permissions());
    let mut access_denied = false;
    let result = run(&fixture, target, None, "restore-first-installed", || {
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o000)).unwrap();
        access_denied = fs::File::open(&parent).is_err();
    });
    drop(reset);
    if !access_denied {
        eprintln!("privileged account bypasses directory access denial");
        return;
    }
    let failed = result.get("failed").unwrap();
    assert_eq!(failed.get("path").unwrap().as_str(), Some("a/item.txt"));
    assert!(
        failed
            .get("error")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("durability is not established")
    );
    assert_eq!(result.get("restored").unwrap().as_array().unwrap().len(), 3);
    for path in names {
        assert_eq!(fixture.read(path).unwrap(), b"original\n");
    }
    assert_eq!(
        result.get("undo").unwrap().get("paths"),
        Some(&Value::Array(names.map(Value::string).to_vec()))
    );

    // Inspect before initialization can reconcile the child's pending work.
    let location =
        crate::checkpoints::identity::locate(fixture.home.path(), fixture.root.path(), false)
            .unwrap()
            .unwrap();
    let store = crate::checkpoints::store::Store::open(location).unwrap();
    let pending: Vec<_> = store.index.pending.values().collect();
    assert_eq!(pending.len(), 1, "only the failed parent remains unsynced");
    let operation = pending[0].seq;
    assert_eq!(pending[0].path, "a/item.txt");
    drop(store);

    // Model loss of the unsynced rename. The still-prepared operation must
    // be classified from the actual bytes when another process reopens it.
    fixture.write("a/item.txt", b"before restore\n");
    let workspace = fixture.open();
    let history = index(&workspace);
    assert!(history.pending.is_empty());
    assert_eq!(
        history.outcomes[&operation],
        crate::checkpoints::record::Outcome::NotApplied
    );
    assert_eq!(fixture.read("a/item.txt").unwrap(), b"before restore\n");
    assert_eq!(fixture.read("b/item.txt").unwrap(), b"original\n");
}
