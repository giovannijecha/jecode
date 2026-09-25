use super::*;
use crate::workspace_fixture as support;
use std::{
    fs::{self, File, FileTimes},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
fn recoveries(files: &support::Fixture) -> crate::workspace::RecoveryStore {
    crate::workspace::RecoveryStore::in_store(&crate::state::Store::in_home(&files.home()).unwrap())
        .unwrap()
}

#[test]
fn cancelled_publication_restores_original_and_destination_race_retains_both() {
    for collision in [false, true] {
        let files = support::Fixture::new();
        files.write("notes", "original\n");
        let ws = Workspace::open(&files.0).unwrap();
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(5),
        };
        let change = ws
            .prepare_edit("notes", "original", "approved", &budget)
            .unwrap();
        let result = ws.apply_with(
            change,
            &budget,
            &recoveries(&files),
            None,
            "test",
            (
                || {
                    if collision {
                        files.write("notes", "concurrent\n");
                    } else {
                        cancelled.store(true, Ordering::Release);
                    }
                },
                || {},
            ),
        );
        let error = result.err().unwrap().to_string();
        if files.unsupported_host_filesystem(&error) {
            assert_eq!(
                fs::read_to_string(files.0.join("notes")).unwrap(),
                "original\n"
            );
            assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
            continue;
        }
        assert_eq!(
            fs::read_to_string(files.0.join("notes")).unwrap(),
            if collision {
                "concurrent\n"
            } else {
                "original\n"
            }
        );
        let retained: Vec<_> = fs::read_dir(&files.0)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".jecode-")
            })
            .collect();
        if collision {
            assert_eq!(retained.len(), 1);
            assert_eq!(fs::read_to_string(&retained[0]).unwrap(), "original\n");
            assert!(error.contains("original retained at"));
        } else {
            assert!(retained.is_empty());
            assert!(error.contains("restored"));
        }
    }
}

#[test]
fn interruption_at_stash_and_publication_can_be_reconciled_from_private_state() {
    for after_publication in [false, true] {
        let files = support::Fixture::new();
        files.write("notes", "original\n");
        let ws = Workspace::open(&files.0)
            .unwrap()
            .with_access(super::super::Access::Local);
        let recoveries = recoveries(&files);
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(5),
        };
        let change = ws
            .prepare_edit("notes", "original", "changed", &budget)
            .unwrap();
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ws.apply_with(
                change,
                &budget,
                &recoveries,
                Some("crash-session"),
                "call-1",
                (
                    || {
                        if !after_publication {
                            panic!("simulated interruption after stash")
                        }
                    },
                    || {
                        if after_publication {
                            panic!("simulated interruption after publication")
                        }
                    },
                ),
            )
            .unwrap();
        }));
        assert!(interrupted.is_err());
        let version = recoveries.list().unwrap().pop().unwrap();
        assert_eq!(version.state, "captured");
        assert_eq!(version.session.as_deref(), Some("crash-session"));
        let inspected = recoveries.inspect(&ws, &version.id, &budget).unwrap();
        assert_eq!(
            inspected.target,
            if after_publication {
                "published result"
            } else {
                "absent"
            }
        );
        assert!(inspected.adjacent.starts_with("present"));
        recoveries.repair(&ws, &version.id, &budget).unwrap();
        assert_eq!(
            fs::read_to_string(files.0.join("notes")).unwrap(),
            if after_publication {
                "changed\n"
            } else {
                "original\n"
            }
        );
        assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
        if after_publication {
            recoveries.restore(&ws, &version.id, &budget).unwrap();
            assert_eq!(
                fs::read_to_string(files.0.join("notes")).unwrap(),
                "original\n"
            );
        }
    }
}

#[test]
fn repair_refuses_competing_file_even_when_its_bytes_and_metadata_match() {
    let files = support::Fixture::new();
    files.write("notes", "original\n");
    let ws = Workspace::open(&files.0).unwrap();
    let recoveries = recoveries(&files);
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(5),
    };
    let change = ws
        .prepare_edit("notes", "original", "changed", &budget)
        .unwrap();
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = ws.apply_with(
            change,
            &budget,
            &recoveries,
            None,
            "call",
            (|| panic!("after stash"), || {}),
        );
    }));
    assert!(interrupted.is_err());
    let version = recoveries.list().unwrap().pop().unwrap();
    let (_, parent, name) = ws.change_parent("notes", &budget).unwrap();
    let mut competing = platform::create(&parent.file, &name).unwrap();
    competing.write_all(b"changed\n").unwrap();
    platform::apply_policy(&competing, &version.after_policy).unwrap();
    competing
        .set_times(FileTimes::new().set_modified(version.after_modified))
        .unwrap();
    assert_ne!(
        platform::identity(&competing).unwrap(),
        version.staged_identity
    );
    drop(competing);
    let error = recoveries
        .repair(
            &ws.with_access(super::super::Access::Local),
            &version.id,
            &budget,
        )
        .unwrap_err();
    assert!(error.to_string().contains("conflicts"), "{error}");
    assert_eq!(
        fs::read_to_string(files.0.join("notes")).unwrap(),
        "changed\n"
    );
    assert_eq!(recoveries.get(&version.id).unwrap().state, "captured");
}

#[test]
fn repair_cleans_verified_adjacent_original_after_applied_checkpoint() {
    let files = support::Fixture::new();
    files.write("notes", "original\n");
    let ws = Workspace::open(&files.0).unwrap();
    let recoveries = recoveries(&files);
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(5),
    };
    let change = ws
        .prepare_edit("notes", "original", "changed", &budget)
        .unwrap();
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = ws.apply_with(
            change,
            &budget,
            &recoveries,
            None,
            "call",
            (|| {}, || panic!("after publication")),
        );
    }));
    assert!(interrupted.is_err());
    let mut version = recoveries.list().unwrap().pop().unwrap();
    let published = File::open(files.0.join("notes")).unwrap();
    recoveries
        .record(
            &mut version,
            "applied",
            Some(platform::identity(&published).unwrap()),
        )
        .unwrap();
    assert!(std::path::Path::new(&version.adjacent).exists());
    recoveries
        .repair(
            &ws.with_access(super::super::Access::Local),
            &version.id,
            &budget,
        )
        .unwrap();
    assert!(!std::path::Path::new(&version.adjacent).exists());
    assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
}

#[test]
fn capture_failure_does_not_publish_and_result_checkpoint_failure_reports_applied() {
    let files = support::Fixture::new();
    files.write("notes", "original\n");
    let ws = Workspace::open(&files.0).unwrap();
    let recoveries = self::recoveries(&files);
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(5),
    };
    let change = ws
        .prepare_edit("notes", "original", "changed", &budget)
        .unwrap();
    fs::remove_dir(recoveries.root()).unwrap();
    let error = ws
        .apply(change, &budget, &recoveries, Some("s"), "capture")
        .err()
        .unwrap();
    assert!(error.to_string().contains("before publication"));
    assert_eq!(
        fs::read_to_string(files.0.join("notes")).unwrap(),
        "original\n"
    );
    assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);

    let recoveries = self::recoveries(&files);
    let change = ws
        .prepare_edit("notes", "original", "changed", &budget)
        .unwrap();
    recoveries.fail_next_capture();
    let incomplete_error = ws
        .apply(
            change,
            &budget,
            &recoveries,
            Some("s"),
            "incomplete-capture",
        )
        .err()
        .unwrap();
    let incomplete = recoveries.list().unwrap().pop().unwrap();
    assert_eq!(incomplete_error.1.as_deref(), Some(incomplete.id.as_str()));
    assert_eq!(incomplete.state, "capturing");
    assert!(recoveries.original(&incomplete.id).is_err());
    assert_eq!(
        fs::read_to_string(files.0.join("notes")).unwrap(),
        "original\n"
    );
    assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);

    let change = ws
        .prepare_edit("notes", "original", "changed", &budget)
        .unwrap();
    recoveries.fail_next_record();
    let applied = ws
        .apply(change, &budget, &recoveries, Some("s"), "record")
        .unwrap();
    assert!(applied.stop_after);
    assert!(applied.warning.unwrap().contains("checkpoint failed"));
    assert_eq!(
        fs::read_to_string(files.0.join("notes")).unwrap(),
        "changed\n"
    );
    assert_eq!(fs::read_dir(&files.0).unwrap().count(), 2);
    let id = applied.recovery.unwrap();
    assert_eq!(recoveries.get(&id).unwrap().state, "captured");
    recoveries
        .repair(&ws.with_access(super::super::Access::Local), &id, &budget)
        .unwrap();
    assert_eq!(recoveries.get(&id).unwrap().state, "applied");
    assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
}
