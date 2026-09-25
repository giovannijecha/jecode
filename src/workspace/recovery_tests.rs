use super::*;
use crate::workspace_fixture::Fixture;
use std::{
    fs,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

fn budget(cancelled: &AtomicBool) -> Budget<'_> {
    Budget {
        cancelled,
        deadline: Instant::now() + Duration::from_secs(60),
    }
}
fn store(files: &Fixture) -> RecoveryStore {
    RecoveryStore::in_store(&crate::state::Store::in_home(&files.home()).unwrap()).unwrap()
}
fn edit(
    ws: &Workspace,
    store: &RecoveryStore,
    old: &str,
    new: &str,
    session: &str,
    operation: &str,
) -> String {
    let cancelled = AtomicBool::new(false);
    let change = ws
        .prepare_edit("index.html", old, new, &budget(&cancelled))
        .unwrap();
    ws.apply(change, &budget(&cancelled), store, Some(session), operation)
        .unwrap()
        .recovery
        .unwrap()
}
#[test]
fn repeated_edits_keep_exact_versions_outside_workspace_and_restore_after_reopen() {
    let files = Fixture::new();
    files.write("index.html", b"<p>zero\r\nend");
    let original_modified = fs::metadata(files.0.join("index.html"))
        .unwrap()
        .modified()
        .unwrap();
    let original_policy =
        platform::capture_policy(&fs::File::open(files.0.join("index.html")).unwrap()).unwrap();
    let ws = Workspace::open(&files.0).unwrap();
    let recoveries = store(&files);
    let first = edit(&ws, &recoveries, "zero", "one", "session-a", "call-1");
    let second = edit(&ws, &recoveries, "one", "two", "session-a", "call-2");
    let third = edit(&ws, &recoveries, "two", "three", "session-b", "call-3");
    assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
    drop(recoveries);
    drop(ws);
    let reopened = store(&files);
    let ws = Workspace::open(&files.0)
        .unwrap()
        .with_access(Access::Local);
    let versions = reopened.list().unwrap();
    assert_eq!(versions.len(), 3);
    for (id, original, session, operation) in [
        (&first, b"<p>zero\r\nend".as_slice(), "session-a", "call-1"),
        (&second, b"<p>one\r\nend".as_slice(), "session-a", "call-2"),
        (&third, b"<p>two\r\nend".as_slice(), "session-b", "call-3"),
    ] {
        let version = reopened.get(id).unwrap();
        assert_eq!(version.workspace, ws.path().to_str().unwrap());
        assert_eq!(
            version.target,
            ws.path().join("index.html").to_str().unwrap()
        );
        assert_eq!(version.session.as_deref(), Some(session));
        assert_eq!(version.operation, operation);
        assert_eq!(
            fs::read(reopened.root().join(format!("{id}.before"))).unwrap(),
            original
        );
        assert_eq!(version.state, "applied");
    }
    let cancelled = AtomicBool::new(false);
    assert!(reopened.restore(&ws, &first, &budget(&cancelled)).is_err());
    reopened.restore(&ws, &third, &budget(&cancelled)).unwrap();
    assert_eq!(
        fs::read(files.0.join("index.html")).unwrap(),
        b"<p>two\r\nend"
    );
    reopened.restore(&ws, &second, &budget(&cancelled)).unwrap();
    reopened.restore(&ws, &first, &budget(&cancelled)).unwrap();
    assert_eq!(
        fs::read(files.0.join("index.html")).unwrap(),
        b"<p>zero\r\nend"
    );
    assert_eq!(
        fs::metadata(files.0.join("index.html"))
            .unwrap()
            .modified()
            .unwrap(),
        original_modified
    );
    let restored_policy =
        platform::capture_policy(&fs::File::open(files.0.join("index.html")).unwrap()).unwrap();
    #[cfg(not(windows))]
    assert_eq!(restored_policy, original_policy);
    #[cfg(windows)]
    {
        // Windows can normalize the auto-inherited marker while retaining the
        // DACL entries and protection mode applied by this transaction path.
        assert_eq!(&restored_policy[4..], &original_policy[4..]);
        assert_eq!(restored_policy[3] & 0x10, original_policy[3] & 0x10);
    }
    assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
}

#[test]
fn conflict_and_same_named_targets_in_distinct_workspaces_never_collide() {
    let files = Fixture::new();
    fs::create_dir(files.0.join("a")).unwrap();
    fs::create_dir(files.0.join("b")).unwrap();
    files.write("a/index.html", "original a");
    files.write("b/index.html", "original b");
    let recoveries = store(&files);
    let a = Workspace::open(&files.0.join("a")).unwrap();
    let b = Workspace::open(&files.0.join("b")).unwrap();
    let cancelled = AtomicBool::new(false);
    let one = a
        .prepare_edit("index.html", "original", "changed", &budget(&cancelled))
        .unwrap();
    let two = b
        .prepare_edit("index.html", "original", "changed", &budget(&cancelled))
        .unwrap();
    let id_a = a
        .apply(
            one,
            &budget(&cancelled),
            &recoveries,
            Some("same-session"),
            "call",
        )
        .unwrap()
        .recovery
        .unwrap();
    let id_b = b
        .apply(
            two,
            &budget(&cancelled),
            &recoveries,
            Some("same-session"),
            "call",
        )
        .unwrap()
        .recovery
        .unwrap();
    assert_ne!(id_a, id_b);
    assert_ne!(
        recoveries.get(&id_a).unwrap().target,
        recoveries.get(&id_b).unwrap().target
    );
    assert!(
        recoveries
            .inspect(&b.with_access(Access::Local), &id_a, &budget(&cancelled))
            .is_err()
    );
    files.write("a/index.html", "user changed this");
    assert!(
        recoveries
            .restore(&a.with_access(Access::Local), &id_a, &budget(&cancelled))
            .is_err()
    );
    assert_eq!(
        fs::read(files.0.join("a/index.html")).unwrap(),
        b"user changed this"
    );
    assert_eq!(
        fs::read(recoveries.root().join(format!("{id_a}.before"))).unwrap(),
        b"original a"
    );
}

#[test]
fn large_edit_uses_streaming_snapshot_and_private_copies() {
    let files = Fixture::new();
    let source = format!(
        "{}UNIQUE\r\n{}",
        "a".repeat(6_000_000),
        "z".repeat(2_000_000)
    );
    files.write("index.html", &source);
    let ws = Workspace::open(&files.0).unwrap();
    let recoveries = store(&files);
    let cancelled = AtomicBool::new(false);
    let change = ws
        .prepare_edit("index.html", "UNIQUE", "changed", &budget(&cancelled))
        .unwrap();
    assert!(matches!(
        change.before.as_ref().unwrap().original,
        snapshot::Original::Staged(_)
    ));
    let id = ws
        .apply(
            change,
            &budget(&cancelled),
            &recoveries,
            Some("large"),
            "edit",
        )
        .unwrap()
        .recovery
        .unwrap();
    assert_eq!(
        fs::metadata(recoveries.root().join(format!("{id}.before")))
            .unwrap()
            .len(),
        source.len() as u64
    );
    assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
}
