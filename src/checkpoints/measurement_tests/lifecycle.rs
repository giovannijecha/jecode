//! Physical collection of a targeted recovery group after acceptance and
//! expiry. An aged acceptance record simulates passage of time without a wait.
use super::capture::close;
use super::*;
use crate::cancel::Cancellation;
use crate::checkpoints::gc;
use crate::checkpoints::identity;
use crate::checkpoints::record::{Record, Recovery, RecoveryAction};
use crate::checkpoints::recovery;

const VERSION_BYTES: usize = 80 << 10;
const VERSIONS: u64 = 20;

pub fn run(recorder: &mut Recorder) {
    let mut accepted_ms = Vec::new();
    let mut expired_ms = Vec::new();
    let mut accepted_before = Vec::new();
    let mut accepted_after = Vec::new();
    let mut expired_before = Vec::new();
    let mut expired_after = Vec::new();
    for _ in 0..recorder.count(3) {
        let (ms, before, after) = collect_group(false);
        accepted_ms.push(ms);
        accepted_before.push(before);
        accepted_after.push(after);

        let (ms, before, after) = collect_group(true);
        expired_ms.push(ms);
        expired_before.push(before);
        expired_after.push(after);
    }
    recorder.add(
        ["retention", "accepted group: physical collection", "total"],
        "20 distinct 80 KiB preimages; group accepted before timed GC; old generation removed before size check",
        accepted_ms,
    );
    recorder.add(
        ["retention", "expired group: physical collection", "total"],
        "accepted deadline set in synthetic past before timed GC; no 24-hour wait",
        expired_ms,
    );
    for (case, before, after) in [
        ("accepted group", accepted_before, accepted_after),
        ("expired group", expired_before, expired_after),
    ] {
        recorder.metric(
            "retention",
            case,
            "median archive bytes before",
            median(before) as f64,
            "bytes",
        );
        recorder.metric(
            "retention",
            case,
            "median archive bytes after",
            median(after) as f64,
            "bytes",
        );
    }
}

fn collect_group(expired: bool) -> (f64, u64, u64) {
    let home = Directory::new();
    let root = Directory::new();
    let target = root.path().join("work.bin");
    fs::write(&target, binary(0, VERSION_BYTES)).unwrap();
    let workspace = workspace::open(home.path(), root.path(), Options::offline()).unwrap();
    ready(&workspace);
    let origin = if expired {
        "measure-expired"
    } else {
        "measure-accepted"
    };
    for seed in 1..=VERSIONS {
        let command = workspace.command_scoped_cancellable(
            origin,
            "synthetic edit",
            &["work.bin".into()],
            &Cancellation::default(),
        );
        fs::write(&target, binary(seed, VERSION_BYTES)).unwrap();
        let report = command.finish();
        assert_eq!(report.status(), "saved", "{report:?}");
    }
    let group = recovery::group_for(&workspace.store().index, origin).unwrap();
    workspace
        .store()
        .transaction(true, |transaction| {
            if expired {
                let now = crate::checkpoints::clock::now();
                let seq = transaction.seq();
                transaction.record(Record::Recovery(Recovery::Status {
                    seq,
                    group,
                    action: RecoveryAction::Accepted,
                    time: now.saturating_sub(recovery::MAX_ACCEPTED_MS + 1),
                    until: now.saturating_sub(1),
                }));
                Ok(())
            } else {
                recovery::accept(transaction, group, origin)
            }
        })
        .unwrap();
    let location = identity::locate(home.path(), root.path(), false)
        .unwrap()
        .unwrap();
    let before = archive_bytes(&workspace.archive);
    let (collected, ms) = timed(|| gc::collect(&location, false).unwrap().unwrap());
    assert!(collected.after < collected.before / 2, "{collected:?}");
    workspace.store().maintain().unwrap();
    let after = archive_bytes(&workspace.archive);
    assert!(
        after < before / 2,
        "archive did not physically shrink: {before} -> {after}"
    );
    if expired {
        assert!(!workspace.store().index.recovery.groups.contains_key(&group));
    } else {
        assert!(workspace.store().index.recovery.groups.contains_key(&group));
    }
    close(workspace);
    (ms, before, after)
}

fn median(mut bytes: Vec<u64>) -> u64 {
    bytes.sort_unstable();
    bytes[bytes.len() / 2]
}
