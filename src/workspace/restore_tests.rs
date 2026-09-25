use super::*;
use crate::workspace_fixture::Fixture;
use std::{
    fs,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

fn interrupted_before_restore_publication() -> (Fixture, RecoveryStore, Workspace, String) {
    let files = Fixture::new();
    files.write("notes", "before\n");
    let store = crate::state::Store::in_home(&files.home()).unwrap();
    let recoveries = RecoveryStore::in_store(&store).unwrap();
    let ws = Workspace::open(&files.0)
        .unwrap()
        .with_access(super::super::Access::Local);
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let change = ws
        .prepare_edit("notes", "before", "after", &budget)
        .unwrap();
    let id = ws
        .apply(change, &budget, &recoveries, Some("s"), "edit")
        .unwrap()
        .recovery
        .unwrap();
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        recoveries
            .restore_with(&ws, &id, &budget, || panic!("after restore stash"), || {})
            .unwrap();
    }));
    assert!(interrupted.is_err());
    assert!(!files.0.join("notes").exists());
    (files, recoveries, ws, id)
}

fn record_orphan_repair_stage(
    files: &Fixture,
    recoveries: &RecoveryStore,
    ws: &Workspace,
    id: &str,
    budget: &Budget<'_>,
) -> std::path::PathBuf {
    let mut version = recoveries.get(id).unwrap();
    let name = ".jecode-staging-900-1";
    let path = files.0.join(name);
    let (_, parent, _) = ws.change_parent(&version.target, budget).unwrap();
    let stage = platform::create(&parent.file, name).unwrap();
    platform::apply_policy(&stage, &version.policy).unwrap();
    use std::io::Write;
    (&stage).write_all(b"before\n").unwrap();
    stage
        .set_times(FileTimes::new().set_modified(version.before_modified))
        .unwrap();
    stage.sync_all().unwrap();
    assert_eq!(
        stage.metadata().unwrap().modified().unwrap(),
        version.before_modified
    );
    version.restore_identity = Some(platform::identity(&stage).unwrap());
    version.restore_stage = Some(name.into());
    let previous = version.published_identity;
    recoveries
        .record(&mut version, "restoring", previous)
        .unwrap();
    path
}

#[test]
fn same_length_corrupt_original_is_not_restored() {
    let files = Fixture::new();
    files.write("notes", "before\n");
    let store = crate::state::Store::in_home(&files.home()).unwrap();
    let recoveries = RecoveryStore::in_store(&store).unwrap();
    let ws = Workspace::open(&files.0)
        .unwrap()
        .with_access(super::super::Access::Local);
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let change = ws
        .prepare_edit("notes", "before", "after", &budget)
        .unwrap();
    let id = ws
        .apply(change, &budget, &recoveries, Some("s"), "edit")
        .unwrap()
        .recovery
        .unwrap();
    fs::write(recoveries.root().join(format!("{id}.before")), b"BROKEN\n").unwrap();
    let result = recoveries.restore(&ws, &id, &budget);
    assert!(result.is_err(), "corrupted private original was restored");
    assert_eq!(fs::read(files.0.join("notes")).unwrap(), b"after\n");
}

#[test]
fn corrupt_result_and_truncated_original_block_recovery_without_mutation() {
    for suffix in ["before", "after"] {
        let files = Fixture::new();
        files.write("notes", "before\n");
        let store = crate::state::Store::in_home(&files.home()).unwrap();
        let recoveries = RecoveryStore::in_store(&store).unwrap();
        let ws = Workspace::open(&files.0)
            .unwrap()
            .with_access(super::super::Access::Local);
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(10),
        };
        let change = ws
            .prepare_edit("notes", "before", "after", &budget)
            .unwrap();
        let id = ws
            .apply(change, &budget, &recoveries, Some("s"), "edit")
            .unwrap()
            .recovery
            .unwrap();
        let retained = recoveries.root().join(format!("{id}.{suffix}"));
        fs::write(
            &retained,
            if suffix == "before" {
                b"bad".as_slice()
            } else {
                b"WRONG\n".as_slice()
            },
        )
        .unwrap();
        let inspected = recoveries.inspect(&ws, &id, &budget).unwrap();
        assert_eq!(
            if suffix == "before" {
                inspected.before_integrity
            } else {
                inspected.after_integrity
            },
            "failed or unavailable"
        );
        assert!(recoveries.restore(&ws, &id, &budget).is_err());
        assert!(recoveries.repair(&ws, &id, &budget).is_err());
        assert_eq!(fs::read(files.0.join("notes")).unwrap(), b"after\n");
        assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
    }
}

#[test]
fn legacy_manifest_can_be_inspected_but_not_trusted_for_automatic_recovery() {
    let files = Fixture::new();
    files.write("notes", "before\n");
    let store = crate::state::Store::in_home(&files.home()).unwrap();
    let recoveries = RecoveryStore::in_store(&store).unwrap();
    let ws = Workspace::open(&files.0)
        .unwrap()
        .with_access(super::super::Access::Local);
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let change = ws
        .prepare_edit("notes", "before", "after", &budget)
        .unwrap();
    let id = ws
        .apply(change, &budget, &recoveries, Some("s"), "edit")
        .unwrap()
        .recovery
        .unwrap();
    let manifest = recoveries.root().join(format!("{id}.json"));
    let mut value =
        crate::json::parse(&fs::read_to_string(&manifest).unwrap(), Default::default()).unwrap();
    let crate::json::Value::Object(ref mut fields) = value else {
        panic!("manifest must be an object")
    };
    fields.insert("version".into(), crate::json::Value::Number("1".into()));
    fields.remove("before_sha256");
    fields.remove("after_sha256");
    fields.remove("replaced_identity");
    fs::write(&manifest, crate::json::encode(&value, 512 * 1024).unwrap()).unwrap();
    let inspected = recoveries.inspect(&ws, &id, &budget).unwrap();
    assert_eq!(inspected.before_integrity, "unverified legacy");
    assert_eq!(inspected.after_integrity, "unverified legacy");
    assert_eq!(
        fs::read(recoveries.root().join(format!("{id}.before"))).unwrap(),
        b"before\n"
    );
    let error = recoveries.restore(&ws, &id, &budget).err().unwrap();
    assert!(
        error
            .to_string()
            .contains("predates capture-time integrity")
    );
    assert!(recoveries.repair(&ws, &id, &budget).is_err());
    assert_eq!(fs::read(files.0.join("notes")).unwrap(), b"after\n");
    assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
}

#[test]
fn repair_retries_after_its_result_checkpoint_fails() {
    let files = Fixture::new();
    files.write("notes", "before\n");
    let store = crate::state::Store::in_home(&files.home()).unwrap();
    let recoveries = RecoveryStore::in_store(&store).unwrap();
    let ws = Workspace::open(&files.0)
        .unwrap()
        .with_access(super::super::Access::Local);
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let change = ws
        .prepare_edit("notes", "before", "after", &budget)
        .unwrap();
    let id = ws
        .apply(change, &budget, &recoveries, Some("s"), "edit")
        .unwrap()
        .recovery
        .unwrap();
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        recoveries
            .restore_with(&ws, &id, &budget, || panic!("isolated interruption"), || {})
            .unwrap();
    }));
    assert!(interrupted.is_err());
    assert!(!files.0.join("notes").exists());
    recoveries.fail_record_after(1);
    let checkpoint = recoveries.repair(&ws, &id, &budget).err().unwrap();
    assert!(
        checkpoint
            .to_string()
            .contains("original restored at target but checkpoint failed")
    );
    assert_eq!(fs::read(files.0.join("notes")).unwrap(), b"before\n");
    let retry = recoveries.repair(&ws, &id, &budget);
    assert!(
        retry.is_ok(),
        "repair rejected its published result: {retry:?}"
    );
    assert_eq!(recoveries.get(&id).unwrap().state, "restored");
    assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
}

#[test]
fn repair_retries_after_intent_checkpoint_failure() {
    let (files, recoveries, ws, id) = interrupted_before_restore_publication();
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let adjacent = recoveries.get(&id).unwrap().adjacent;
    recoveries.fail_next_record();
    assert!(recoveries.repair(&ws, &id, &budget).is_err());
    assert!(!files.0.join("notes").exists());
    assert!(Path::new(&adjacent).exists());
    assert_eq!(recoveries.get(&id).unwrap().state, "restoring");
    recoveries.repair(&ws, &id, &budget).unwrap();
    assert_eq!(fs::read(files.0.join("notes")).unwrap(), b"before\n");
    assert!(!Path::new(&adjacent).exists());
}

#[test]
fn repair_removes_only_its_verified_orphan_stage_after_interruption() {
    let (files, recoveries, ws, id) = interrupted_before_restore_publication();
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let stage = record_orphan_repair_stage(&files, &recoveries, &ws, &id, &budget);
    assert!(stage.exists());
    recoveries.repair(&ws, &id, &budget).unwrap();
    assert!(!stage.exists());
    assert_eq!(fs::read(files.0.join("notes")).unwrap(), b"before\n");
    assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);

    let (files, recoveries, ws, id) = interrupted_before_restore_publication();
    let stage = record_orphan_repair_stage(&files, &recoveries, &ws, &id, &budget);
    let replacement = files.0.join("replacement");
    fs::write(&replacement, b"before\n").unwrap();
    fs::remove_file(&stage).unwrap();
    fs::rename(replacement, &stage).unwrap();
    let error = recoveries.repair(&ws, &id, &budget).err().unwrap();
    assert!(error.to_string().contains("restoration stage changed"));
    assert_eq!(fs::read(&stage).unwrap(), b"before\n");
    assert!(!files.0.join("notes").exists());
    assert!(Path::new(&recoveries.get(&id).unwrap().adjacent).exists());
}

#[test]
fn repair_interruption_at_each_own_boundary_is_idempotently_reconciled() {
    for boundary in 0..3 {
        let (files, recoveries, ws, id) = interrupted_before_restore_publication();
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(10),
        };
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            recoveries
                .repair_with(
                    &ws,
                    &id,
                    &budget,
                    (
                        || {
                            if boundary == 0 {
                                panic!("after repair intent")
                            }
                        },
                        || {
                            if boundary == 1 {
                                panic!("after repair publication")
                            }
                        },
                        || {
                            if boundary == 2 {
                                panic!("after repair result checkpoint")
                            }
                        },
                    ),
                )
                .unwrap();
        }));
        assert!(interrupted.is_err());
        let recorded = recoveries.get(&id).unwrap();
        assert_eq!(
            recorded.state,
            if boundary == 2 {
                "restored"
            } else {
                "restoring"
            }
        );
        assert!(Path::new(&recorded.adjacent).exists());
        if boundary == 0 {
            assert!(!files.0.join("notes").exists());
        } else {
            assert_eq!(fs::read(files.0.join("notes")).unwrap(), b"before\n");
        }
        recoveries.repair(&ws, &id, &budget).unwrap();
        let target = fs::File::open(files.0.join("notes")).unwrap();
        let identity = platform::identity(&target).unwrap();
        assert_eq!(recoveries.get(&id).unwrap().state, "restored");
        assert!(!Path::new(&recorded.adjacent).exists());
        assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
        recoveries.repair(&ws, &id, &budget).unwrap();
        assert_eq!(
            platform::identity(&fs::File::open(files.0.join("notes")).unwrap()).unwrap(),
            identity
        );
    }
}

#[test]
fn corrupt_result_cannot_authorize_restored_state_cleanup() {
    let (files, recoveries, ws, id) = interrupted_before_restore_publication();
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        recoveries
            .repair_with(
                &ws,
                &id,
                &budget,
                (|| {}, || {}, || panic!("after restored checkpoint")),
            )
            .unwrap();
    }));
    assert!(interrupted.is_err());
    let recorded = recoveries.get(&id).unwrap();
    assert_eq!(recorded.state, "restored");
    assert!(Path::new(&recorded.adjacent).exists());
    fs::write(recoveries.root().join(format!("{id}.after")), b"WRONG\n").unwrap();
    assert!(recoveries.repair(&ws, &id, &budget).is_err());
    assert!(Path::new(&recorded.adjacent).exists());
    assert_eq!(fs::read(files.0.join("notes")).unwrap(), b"before\n");
    assert_eq!(fs::read(&recorded.adjacent).unwrap(), b"after\n");
}

#[test]
fn repair_preserves_replaced_adjacent_and_competing_target() {
    let (files, recoveries, ws, id) = interrupted_before_restore_publication();
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let adjacent = recoveries.get(&id).unwrap().adjacent;
    files.write("candidate", "after\n");
    fs::remove_file(&adjacent).unwrap();
    fs::rename(files.0.join("candidate"), &adjacent).unwrap();
    assert!(recoveries.repair(&ws, &id, &budget).is_err());
    assert!(!files.0.join("notes").exists());
    assert_eq!(fs::read(&adjacent).unwrap(), b"after\n");

    let files = Fixture::new();
    files.write("notes", "before\n");
    let store = crate::state::Store::in_home(&files.home()).unwrap();
    let recoveries = RecoveryStore::in_store(&store).unwrap();
    let ws = Workspace::open(&files.0)
        .unwrap()
        .with_access(super::super::Access::Local);
    let change = ws
        .prepare_edit("notes", "before", "after", &budget)
        .unwrap();
    let id = ws
        .apply(change, &budget, &recoveries, Some("s"), "edit")
        .unwrap()
        .recovery
        .unwrap();
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        recoveries
            .restore_with(
                &ws,
                &id,
                &budget,
                || {
                    // The intended stage still exists, so the competing file
                    // cannot reuse its native identity.
                    files.write("notes", "before\n");
                    panic!("competing target")
                },
                || {},
            )
            .unwrap();
    }));
    assert!(interrupted.is_err());
    assert!(recoveries.repair(&ws, &id, &budget).is_err());
    assert_eq!(fs::read(files.0.join("notes")).unwrap(), b"before\n");
    assert_eq!(recoveries.get(&id).unwrap().state, "restoring");
    assert!(Path::new(&recoveries.get(&id).unwrap().adjacent).exists());
}

#[test]
fn restoration_interruption_at_each_effect_boundary_is_repairable() {
    for after_publication in [false, true] {
        let files = Fixture::new();
        files.write("notes", "before\n");
        let store = crate::state::Store::in_home(&files.home()).unwrap();
        let recoveries = RecoveryStore::in_store(&store).unwrap();
        let ws = Workspace::open(&files.0)
            .unwrap()
            .with_access(super::super::Access::Local);
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(10),
        };
        let change = ws
            .prepare_edit("notes", "before", "after", &budget)
            .unwrap();
        let id = ws
            .apply(change, &budget, &recoveries, Some("s"), "edit")
            .unwrap()
            .recovery
            .unwrap();
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            recoveries
                .restore_with(
                    &ws,
                    &id,
                    &budget,
                    || {
                        if !after_publication {
                            panic!("restore interrupted after stash")
                        }
                    },
                    || {
                        if after_publication {
                            panic!("restore interrupted after publication")
                        }
                    },
                )
                .unwrap();
        }));
        assert!(interrupted.is_err());
        assert_eq!(recoveries.get(&id).unwrap().state, "restoring");
        let inspected = recoveries.inspect(&ws, &id, &budget).unwrap();
        assert_eq!(
            inspected.target,
            if after_publication {
                "retained original"
            } else {
                "absent"
            }
        );
        assert!(inspected.adjacent.starts_with("present"));
        recoveries.repair(&ws, &id, &budget).unwrap();
        assert_eq!(
            fs::read_to_string(files.0.join("notes")).unwrap(),
            "before\n"
        );
        assert_eq!(recoveries.get(&id).unwrap().state, "restored");
        assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
    }
}
