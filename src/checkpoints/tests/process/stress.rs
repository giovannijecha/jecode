//! A real process is stopped only after it signals a durable checkpoint stage.
use super::{HOME, MODE, ROOT};
use crate::checkpoints::identity;
use crate::checkpoints::mutation::Protection;
use crate::checkpoints::quota;
use crate::checkpoints::record::{Outcome, State};
use crate::checkpoints::restore::Expected;
use crate::checkpoints::store::Store;
use crate::checkpoints::workspace::{self, Options};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const STAGE: &str = "JECODE_CHECKPOINT_TEST_STAGE";
const TARGET: &str = "JECODE_CHECKPOINT_TEST_TARGET";
const TIMEOUT: Duration = Duration::from_secs(90);
const FILE_BYTES: usize = 80 << 10;
const HEADROOM: u64 = 4 << 20;

fn file(number: usize) -> String {
    format!("file-{number}.bin")
}

fn bytes(number: usize, initial: bool) -> Vec<u8> {
    super::super::noise(
        (if initial { 100 } else { 200 }) + number as u64,
        FILE_BYTES,
    )
}

fn marker(home: &Path, name: &str, suffix: &str) -> PathBuf {
    home.join(format!("checkpoint-{name}.{suffix}"))
}

pub(super) fn stage(name: &str) {
    if !std::env::var(STAGE)
        .ok()
        .is_some_and(|stages| stages.split(',').any(|stage| stage == name))
    {
        return;
    }
    let home = PathBuf::from(std::env::var(HOME).unwrap());
    let ready = marker(&home, name, "ready");
    let release = marker(&home, name, "release");
    let mut signal = File::create(ready).unwrap();
    signal.write_all(b"ready").unwrap();
    signal.sync_all().unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while !release.exists() {
        assert!(Instant::now() < deadline, "stage {name} was not released");
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn child(home: &Path, root: &Path, mode: &str) {
    let workspace = workspace::open(home, root, Options::offline()).unwrap();
    workspace.initialize().unwrap();
    if let Some(number) = mode.strip_prefix("stress-write-") {
        let number: usize = number.parse().unwrap();
        let path = root.join(file(number));
        for revision in 0..3 {
            let content = if revision == 2 {
                bytes(number, false)
            } else {
                super::super::noise(300 + number as u64 * 10 + revision, FILE_BYTES)
            };
            let mut mutation = workspace.mutation(mode, "write", &path);
            mutation.prepare(&content);
            fs::write(&path, &content).unwrap();
            assert!(matches!(
                mutation.finish(true),
                Protection::Protected { .. }
            ));
        }
        workspace
            .mark("milestone", mode, mode, false, false)
            .unwrap();
        return;
    }
    match mode {
        "stress-gc" => {
            assert!(workspace.collect(true).unwrap().is_some());
        }
        "stress-restore" => {
            let target: u64 = std::env::var(TARGET).unwrap().parse().unwrap();
            let report = workspace
                .restore(&super::super::request(
                    &workspace,
                    target,
                    None,
                    Expected::Latest,
                ))
                .unwrap();
            assert!(report.failed.is_none() && report.conflicts.is_empty());
        }
        _ => panic!("unexpected stress child mode {mode}"),
    }
}

struct Session(Option<Child>);

impl Session {
    fn spawn(
        fixture: &super::super::Fixture,
        mode: &str,
        stage: Option<&str>,
        target: u64,
    ) -> Self {
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
            .env(TARGET, target.to_string())
            .env_remove(STAGE)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(stage) = stage {
            command.env(STAGE, stage);
        }
        Self(Some(command.spawn().unwrap()))
    }

    fn ready(&mut self, fixture: &super::super::Fixture, stage: &str) {
        let ready = marker(fixture.home.path(), stage, "ready");
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if ready.exists() {
                return;
            }
            let child = self.0.as_mut().unwrap();
            if let Some(status) = child.try_wait().unwrap() {
                panic!("child exited before {stage}: {status}");
            }
            assert!(Instant::now() < deadline, "child did not reach {stage}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn finish(mut self) {
        let deadline = Instant::now() + TIMEOUT;
        while self.0.as_mut().unwrap().try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "child did not finish");
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = self.0.take().unwrap().wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn kill(&mut self) {
        let mut child = self.0.take().unwrap();
        child.kill().unwrap();
        assert!(!child.wait().unwrap().success());
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn assert_bytes(fixture: &super::super::Fixture, initial: &[bool; 4]) {
    for (number, initial) in initial.iter().enumerate() {
        assert_eq!(
            fixture.read(&file(number)).unwrap(),
            bytes(number, *initial)
        );
    }
}

fn assert_capacity(archive: &Path) {
    let used = quota::used(archive).unwrap();
    assert!(
        used <= quota::LIMIT,
        "archive used {used} of {}",
        quota::LIMIT
    );
}

#[test]
fn four_sessions_survive_concurrent_collection_and_crashed_restore_near_capacity() {
    let fixture = super::super::Fixture::new();
    for number in 0..4 {
        fixture.write(&file(number), &bytes(number, true));
    }
    let workspace = fixture.open();
    workspace
        .configure(Some(4 << 20), Some(128 << 10), "test")
        .unwrap();
    let baseline = workspace
        .store()
        .index
        .points
        .values()
        .find(|point| point.kind == "baseline")
        .unwrap()
        .seq;
    workspace.pin(baseline, "retained baseline", true).unwrap();
    let archive = workspace.archive.clone();
    drop(workspace);

    // A single logical filler approaches the real 1 GiB hard cap. The journal
    // itself is small; three revisions per file exercise retention separately.
    // The filler lives only in this isolated archive and is never copied.
    let filler = File::create(archive.join("test-owned-filler.bin")).unwrap();
    let used = quota::used(&archive).unwrap();
    filler.set_len(quota::LIMIT - HEADROOM - used).unwrap();
    drop(filler);
    assert_eq!(quota::used(&archive).unwrap(), quota::LIMIT - HEADROOM);
    assert_capacity(&archive);

    let writers: Vec<_> = (0..3)
        .map(|number| Session::spawn(&fixture, &format!("stress-write-{number}"), None, 0))
        .collect();
    for writer in writers {
        writer.finish();
    }
    assert_capacity(&archive);
    let current = fs::read_to_string(archive.join("CURRENT")).ok();
    let location = identity::locate(fixture.home.path(), fixture.root.path(), false)
        .unwrap()
        .unwrap();
    let old_generation = Store::open(location).unwrap().journal.generation;

    // Collection has already copied one archived object, but not its full
    // snapshot. The fourth independent writer commits while it is paused.
    let mut collector = Session::spawn(
        &fixture,
        "stress-gc",
        Some("gc-first-object-copied,gc-moved"),
        0,
    );
    collector.ready(&fixture, "gc-first-object-copied");
    Session::spawn(&fixture, "stress-write-3", None, 0).finish();
    assert_bytes(&fixture, &[false; 4]);
    assert_capacity(&archive);
    fs::write(
        marker(fixture.home.path(), "gc-first-object-copied", "release"),
        b"go",
    )
    .unwrap();
    // The replacement generation and Moved marker are durable; CURRENT is old.
    // An OS kill here exercises discovery and repair on the next open.
    collector.ready(&fixture, "gc-moved");
    assert_eq!(fs::read_to_string(archive.join("CURRENT")).ok(), current);
    assert_capacity(&archive);
    collector.kill();
    let location = identity::locate(fixture.home.path(), fixture.root.path(), false)
        .unwrap()
        .unwrap();
    let reopened = Store::open(location.clone()).unwrap();
    assert_eq!(reopened.journal.generation, old_generation + 1);
    assert_eq!(
        fs::read_to_string(archive.join("CURRENT")).unwrap(),
        (old_generation + 1).to_string()
    );
    assert!(
        reopened.index.horizon > baseline,
        "collection pruned old revisions"
    );
    assert!(reopened.index.pins.contains_key(&baseline));
    assert!(reopened.index.pending.is_empty());
    assert!(
        reopened
            .index
            .points
            .values()
            .any(|point| point.label == "stress-write-3")
    );
    for number in 0..4 {
        let version = reopened.index.current(&file(number)).unwrap();
        let State::File { object, .. } = &version.state else {
            panic!("writer {number} has no stored file");
        };
        let expected = bytes(number, false);
        assert_eq!(&reopened.bytes(*object).unwrap()[..], expected.as_slice());
    }
    assert_bytes(&fixture, &[false; 4]);
    assert_capacity(&archive);
    drop(reopened);

    let mut restoring = Session::spawn(
        &fixture,
        "stress-restore",
        Some("restore-first-installed"),
        baseline,
    );
    restoring.ready(&fixture, "restore-first-installed");
    let pending = Store::open(location).unwrap().index.pending;
    assert_eq!(pending.len(), 4);
    assert_bytes(&fixture, &[true, false, false, false]);
    assert_capacity(&archive);
    restoring.kill();

    let reopened = fixture.open();
    let index = super::super::index(&reopened);
    assert!(index.pending.is_empty());
    for prepare in pending.values() {
        let expected = if prepare.path == file(0) {
            Outcome::Applied
        } else {
            Outcome::NotApplied
        };
        assert_eq!(index.outcomes[&prepare.seq], expected, "{}", prepare.path);
    }
    let before = index
        .points
        .values()
        .find(|point| point.kind == "restore-before")
        .unwrap()
        .seq;
    reopened
        .restore(&super::super::request(
            &reopened,
            before,
            None,
            Expected::Latest,
        ))
        .unwrap();
    assert_bytes(&fixture, &[false; 4]);
    let restored = reopened
        .restore(&super::super::request(
            &reopened,
            baseline,
            None,
            Expected::Latest,
        ))
        .unwrap();
    assert!(restored.failed.is_none() && restored.conflicts.is_empty());
    assert_bytes(&fixture, &[true; 4]);
    let undo = restored.before.unwrap();
    drop(reopened);
    let reopened = fixture.open();
    reopened
        .restore(&super::super::request(
            &reopened,
            undo,
            None,
            Expected::Latest,
        ))
        .unwrap();
    assert_bytes(&fixture, &[false; 4]);
    assert!(super::super::index(&reopened).pending.is_empty());
    assert_capacity(&archive);
}
