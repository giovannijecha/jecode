//! Several processes share one archive. Children are this test binary
//! re-entered through an ignored test that acts only when asked to.
use super::*;
use crate::checkpoints::mutation::Protection;
use crate::checkpoints::quota;
use crate::checkpoints::record::{Outcome, State};
use crate::checkpoints::store::Store;
use std::path::Path;
use std::process::{Command, Stdio};

const MODE: &str = "JECODE_CHECKPOINT_CHILD";
const HOME: &str = "JECODE_CHECKPOINT_CHILD_HOME";
const ROOT: &str = "JECODE_CHECKPOINT_CHILD_ROOT";
const QUOTA: &str = "JECODE_CHECKPOINT_TEST_QUOTA";
const WRITES: usize = 12;

mod stress;

pub(crate) fn stage(name: &str) {
    stress::stage(name);
}

fn spawn(fixture: &Fixture, mode: &str) -> std::process::Child {
    spawn_limited(fixture, mode, None)
}

fn spawn_limited(fixture: &Fixture, mode: &str, limit: Option<u64>) -> std::process::Child {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "checkpoints::tests::process::child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(MODE, mode)
        .env(HOME, fixture.home.path())
        .env(ROOT, fixture.root.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(limit) = limit {
        command.env(QUOTA, limit.to_string());
    } else {
        command.env_remove(QUOTA);
    }
    command.spawn().unwrap()
}

fn wait(child: std::process::Child) -> String {
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

/// Entry point of a child process; does nothing in a normal test run.
#[test]
#[ignore = "run by the multi-process tests"]
fn child() {
    let Ok(mode) = std::env::var(MODE) else {
        return;
    };
    let home = std::env::var(HOME).unwrap();
    let root = std::env::var(ROOT).unwrap();
    if mode.starts_with("stress-") {
        stress::child(Path::new(&home), Path::new(&root), &mode);
        return;
    }
    let workspace =
        workspace::open(Path::new(&home), Path::new(&root), Options::offline()).unwrap();
    if let Ok(limit) = std::env::var(QUOTA) {
        workspace.store().journal.quota_limit = limit.parse().unwrap();
        let path = format!("{mode}.bin");
        let seed: u64 = mode.strip_prefix("quota-").unwrap().parse().unwrap();
        let bytes = noise(seed, 8 << 10);
        let mutation = {
            let absolute = super::super::identity::absolute(&workspace.root, &path);
            let mut mutation = workspace.mutation(&mode, "write", &absolute);
            mutation.prepare(&bytes);
            fs::write(&absolute, &bytes).unwrap();
            mutation
        };
        let protection = mutation.finish(true);
        assert_eq!(fs::read(workspace.root.join(&path)).unwrap(), bytes);
        println!(
            "quota: {}",
            if matches!(protection, Protection::Protected { .. }) {
                "protected"
            } else {
                "unprotected"
            }
        );
        return;
    }
    workspace.initialize().unwrap();
    let write = |relative: &str, bytes: &[u8]| {
        let path = super::super::identity::absolute(&workspace.root, relative);
        let mut mutation = workspace.mutation(&mode, "write", &path);
        mutation.prepare(bytes);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        mutation
    };
    if mode == "crash" {
        let mutation = write("shared.txt", b"crashed mid-operation\n");
        // Exit without completing, as a killed process would.
        std::mem::forget(mutation);
        std::process::exit(0);
    }
    for index in 0..WRITES {
        let own = write(
            &format!("{mode}/{index}.txt"),
            format!("{mode} {index}\n").as_bytes(),
        );
        assert!(matches!(own.finish(true), Protection::Protected { .. }));
        let shared = write("shared.txt", format!("{mode} {index}\n").as_bytes());
        assert!(matches!(shared.finish(true), Protection::Protected { .. }));
    }
    workspace
        .mark("milestone", &mode, &mode, false, false)
        .unwrap();
    println!("damage: {}", workspace.store().journal.damage.len());
}

fn concurrent_writers(processes: usize) {
    let fixture = Fixture::new();
    fixture.write("shared.txt", b"start\n");
    // The archive and its baseline exist before the writers start.
    drop(fixture.open());
    let children: Vec<_> = (0..processes)
        .map(|number| spawn(&fixture, &format!("writer-{number}")))
        .collect();
    for child in children {
        assert!(wait(child).contains("damage: 0"));
    }

    let workspace = fixture.open();
    let index = index(&workspace);
    assert!(index.pending.is_empty());
    assert!(workspace.store().journal.damage.is_empty());
    let applied = index
        .outcomes
        .values()
        .filter(|outcome| **outcome == Outcome::Applied)
        .count();
    assert_eq!(applied, processes * WRITES * 2);
    for number in 0..processes {
        for write in 0..WRITES {
            let path = format!("writer-{number}/{write}.txt");
            let version = index.current(&path).unwrap();
            assert_eq!(version.origin, format!("writer-{number}"));
            assert_eq!(
                fixture.read(&path).unwrap(),
                format!("writer-{number} {write}\n").as_bytes()
            );
        }
    }
    // Leases serialize the shared file: every write saved exactly the
    // previous writer's result, so no state is missing from the history.
    let versions = &index.paths["shared.txt"];
    assert_eq!(versions.len(), 1 + processes * WRITES);
    assert!(versions[1..].iter().all(|version| version.operation != 0));
    let mut sequences: Vec<u64> = index
        .paths
        .values()
        .flatten()
        .map(|version| version.seq)
        .collect();
    let count = sequences.len();
    sequences.sort_unstable();
    sequences.dedup();
    assert_eq!(sequences.len(), count, "sequence numbers are unique");
}

#[test]
fn one_process_writes_through_a_child() {
    concurrent_writers(1);
}

#[test]
fn two_processes_write_concurrently() {
    concurrent_writers(2);
}

#[test]
fn four_processes_write_concurrently() {
    concurrent_writers(4);
}

#[test]
fn four_processes_share_one_hard_archive_capacity() {
    let fixture = Fixture::new();
    for number in 0..4 {
        fixture.write(&format!("quota-{number}.bin"), b"saved before\n");
    }
    let workspace = fixture.open();
    let archive = workspace.archive.clone();
    drop(workspace);
    fs::write(archive.join("test-owned.bin"), vec![0; 300_000]).unwrap();
    let limit = quota::used(&archive).unwrap() + quota::RESERVE + (24 << 10);
    let children: Vec<_> = (0..4)
        .map(|number| spawn_limited(&fixture, &format!("quota-{number}"), Some(limit)))
        .collect();
    let results: Vec<_> = children.into_iter().map(wait).collect();
    assert!(
        results
            .iter()
            .any(|result| result.contains("quota: protected"))
    );
    assert!(
        results
            .iter()
            .any(|result| result.contains("quota: unprotected"))
    );
    assert!(quota::used(&archive).unwrap() <= limit);

    let location = super::super::identity::locate(fixture.home.path(), fixture.root.path(), false)
        .unwrap()
        .unwrap();
    let store = Store::open(location).unwrap();
    assert!(store.index.pending.is_empty());
    for versions in store.index.paths.values() {
        for version in versions {
            if let State::File { object, .. } = version.state {
                store.bytes(object).unwrap();
            }
        }
    }
    for number in 0..4 {
        assert_eq!(
            fixture.read(&format!("quota-{number}.bin")).unwrap(),
            noise(number, 8 << 10)
        );
    }
}

#[test]
fn an_operation_of_a_killed_process_is_recovered_by_the_next_one() {
    let fixture = Fixture::new();
    fixture.write("shared.txt", b"before\n");
    drop(fixture.open());
    wait(spawn(&fixture, "crash"));

    let workspace = fixture.open();
    let index = index(&workspace);
    assert!(index.pending.is_empty());
    assert_eq!(
        index.outcomes.values().copied().collect::<Vec<_>>(),
        [Outcome::Applied]
    );
    let current = index.current("shared.txt").unwrap();
    assert_eq!(current.origin, "crash");
    let operation = current.operation;
    assert!(matches!(
        index.state_at("shared.txt", operation),
        State::File { length: 7, .. }
    ));
    let selected = paths(&["shared.txt"]);
    workspace
        .restore(&request(
            &workspace,
            operation,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert_eq!(fixture.read("shared.txt").unwrap(), b"before\n");
}

#[test]
fn orderly_store_closes_remove_only_their_owner_locks() {
    let fixture = Fixture::new();
    drop(fixture.open());
    let location = super::super::identity::locate(fixture.home.path(), fixture.root.path(), false)
        .unwrap()
        .unwrap();
    let owners = location.directory.join("owners");
    let live = Store::open(location.clone()).unwrap();
    let live_path = owners.join(format!("{}.lock", live.owner));
    assert!(live_path.exists());
    assert!(!super::super::locks::owner_gone(
        &location.directory,
        &live.owner
    ));

    for _ in 0..16 {
        let other = Store::open(location.clone()).unwrap();
        let other_path = owners.join(format!("{}.lock", other.owner));
        assert!(other_path.exists());
        drop(other);
        assert!(!other_path.exists());
        assert!(live_path.exists());
    }
    assert_eq!(fs::read_dir(&owners).unwrap().count(), 1);
    let live_owner = live.owner.clone();
    drop(live);
    assert!(!live_path.exists());
    assert!(super::super::locks::owner_gone(
        &location.directory,
        &live_owner
    ));
    assert_eq!(fs::read_dir(&owners).unwrap().count(), 0);
}
