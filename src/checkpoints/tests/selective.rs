//! Complete baselines and conservative recovery from incomplete coverage.
use super::*;
use crate::cancel::Cancellation;
use crate::checkpoints::inventory::{self, Limits};
use crate::checkpoints::locks::{Lease, Leases};
use crate::checkpoints::mutation::{Freshness, Protection};
use crate::checkpoints::policy::Policy;
use crate::checkpoints::record::State;
use crate::json::Value;
use std::sync::mpsc;
use std::time::Duration;

fn blocked_baseline(fixture: &Fixture) -> (std::sync::Arc<Workspace>, Lease) {
    fixture.write("source.txt", b"before\n");
    let location =
        crate::checkpoints::identity::locate(fixture.home.path(), fixture.root.path(), true)
            .unwrap()
            .unwrap();
    let leases = Leases::new(crate::checkpoints::layout::leases_directory(
        &location.directory,
    ));
    let held = leases.acquire(&["source.txt"]).unwrap();
    let workspace = workspace::open(
        fixture.home.path(),
        fixture.root.path(),
        workspace::Options {
            watch: false,
            background: true,
            verify: false,
        },
    )
    .unwrap();
    (workspace, held)
}

#[test]
fn first_bash_point_waits_for_the_complete_baseline() {
    let fixture = Fixture::new();
    let (workspace, held) = blocked_baseline(&fixture);
    let (sender, receiver) = mpsc::channel();
    let command_workspace = workspace.clone();
    let command_thread = std::thread::spawn(move || {
        sender
            .send(command_workspace.command("test", "change any file"))
            .unwrap();
    });
    assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());
    assert!(
        index(&workspace)
            .points
            .values()
            .all(|point| point.kind != "command")
    );
    drop(held);
    let command = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    command_thread.join().unwrap();
    assert!(matches!(command.freshness, Freshness::Current));
    assert!(matches!(
        index(&workspace).current_state("source.txt"),
        State::File { .. }
    ));
    assert!(command.finish().before.is_some());
}

#[test]
fn cancelling_a_bash_wait_does_not_create_a_point() {
    let fixture = Fixture::new();
    let (workspace, held) = blocked_baseline(&fixture);
    let cancellation = Cancellation::default();
    let child_cancellation = cancellation.clone();
    let (sender, receiver) = mpsc::channel();
    let command_workspace = workspace.clone();
    let command_thread = std::thread::spawn(move || {
        sender
            .send(command_workspace.command_cancellable(
                "test",
                "change any file",
                &child_cancellation,
            ))
            .unwrap();
    });
    assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());
    cancellation.cancel();
    let command = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
    command_thread.join().unwrap();
    assert!(matches!(command.freshness, Freshness::Unavailable(_)));
    assert!(command.finish().before.is_none());
    assert!(
        index(&workspace)
            .points
            .values()
            .all(|point| point.kind != "command")
    );
    drop(held);
}

#[test]
fn default_scope_restores_a_file_above_ten_mib() {
    let fixture = Fixture::new();
    let original = noise(3, 11 << 20);
    fixture.write("large.bin", &original);
    let workspace = fixture.open();
    assert!(matches!(
        index(&workspace).current_state("large.bin"),
        State::File { length, .. } if length == original.len() as u64
    ));
    let command = workspace.command("test", "replace large file");
    assert!(matches!(command.freshness, Freshness::Current));
    fixture.write("large.bin", b"changed");
    let before = command.finish().before.unwrap();
    let report = workspace
        .restore(&request(&workspace, before, None, Expected::Latest))
        .unwrap();
    assert!(report.failed.is_none() && report.conflicts.is_empty());
    assert_eq!(fixture.read("large.bin"), Some(original));
}

#[test]
fn config_can_restore_the_unlimited_file_default() {
    let fixture = Fixture::new();
    let workspace = fixture.open();
    workspace.configure(None, Some(1024), "test").unwrap();
    let result = crate::checkpoints::actions::run(
        &workspace,
        &crate::checkpoints::actions::Caller {
            origin: "test",
            user: true,
        },
        &Value::object([
            ("action", Value::string("config")),
            ("max_file", Value::string("unlimited")),
        ]),
    )
    .unwrap();
    assert_eq!(workspace.store().policy.max_file_bytes, u64::MAX);
    assert_eq!(
        result.get("max_file").and_then(Value::as_str),
        Some("unlimited")
    );
}

fn small_budget(fixture: &Fixture) {
    let workspace = fixture.open();
    workspace
        .configure(Some(1 << 20), Some(512 << 10), "test")
        .unwrap();
}

fn heavy(fixture: &Fixture) {
    for index in 0..4 {
        fixture.write(&format!("data/{index}.bin"), &noise(index, 384 << 10));
    }
}

#[test]
fn discovery_bounds_visited_paths_including_excluded_names() {
    let fixture = Fixture::new();
    for index in 0..20 {
        fixture.write(&format!("{index}.ignored"), b"");
    }
    let mut policy = Policy::default();
    policy.exclude.push("*.ignored".into());
    let walk = inventory::bounded(
        fixture.root.path(),
        "",
        &policy,
        &[],
        Limits {
            paths: 8,
            bytes: 1 << 20,
        },
    );
    assert!(walk.limited.as_deref().unwrap().contains("8 paths"));
    assert!(walk.entries.is_empty());
    assert_eq!(walk.excluded, 8);
}

#[test]
fn owned_paths_are_excluded_by_components_and_native_casing() {
    let policy = Policy::default();
    let excluded = paths(&["private/home"]);
    assert!(!crate::checkpoints::scan::in_scope(
        &policy,
        &excluded,
        "private/home/file",
        false
    ));
    assert!(crate::checkpoints::scan::in_scope(
        &policy,
        &excluded,
        "private/homework/file",
        false
    ));
    assert_eq!(
        crate::checkpoints::scan::in_scope(&policy, &excluded, "PRIVATE/HOME/file", false),
        !cfg!(windows)
    );
    assert!(!crate::checkpoints::scan::in_scope(
        &policy,
        &paths(&[""]),
        "file",
        false
    ));
}

#[test]
fn generated_subtrees_are_pruned_before_counting_their_contents() {
    let fixture = Fixture::new();
    for index in 0..20 {
        fixture.write(&format!("target/nested/{index}"), b"data");
    }
    fixture.write("source.rs", b"source");
    let walk = inventory::bounded(
        fixture.root.path(),
        "",
        &Policy::default(),
        &[],
        Limits {
            paths: 4,
            bytes: 1 << 20,
        },
    );
    assert!(walk.limited.is_none());
    assert_eq!(walk.excluded, 1);
    assert_eq!(walk.entries.len(), 1);
    assert_eq!(walk.entries[0].path, "source.rs");
}

#[test]
fn baseline_covers_content_above_the_soft_retention_budget() {
    let fixture = Fixture::new();
    small_budget(&fixture);
    heavy(&fixture);
    fixture.write("untouched.txt", b"keep this");
    let workspace = fixture.open();
    assert_eq!(workspace.readiness(), workspace::Readiness::Ready);
    assert!(workspace.selective().is_none());
    assert!(workspace.wait_ready().is_ok());
    assert!(index(&workspace).current("data/0.bin").is_some());
    assert!(index(&workspace).current("untouched.txt").is_some());
    let initial = workspace
        .mark("turn", "complete", "model", false, false)
        .unwrap();
    let collection = workspace.collect(true).unwrap().unwrap();
    assert!(collection.over_budget);
    assert!(index(&workspace).current("data/0.bin").is_some());
    let before = fixture.read("data/0.bin").unwrap();
    let mut mutation = workspace.mutation("model", "write", &fixture.path("data/0.bin"));
    mutation.prepare(b"changed");
    fixture.write("data/0.bin", b"changed");
    let Protection::Protected { before: saved, .. } = mutation.finish(true) else {
        panic!("touched file was not protected");
    };
    assert!(matches!(
        workspace.command("model", "read-only").freshness,
        Freshness::Current
    ));
    let selected = paths(&["data/0.bin"]);
    let report = workspace
        .restore(&request(
            &workspace,
            saved,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert_eq!(report.applied.len(), 1);
    assert_eq!(fixture.read("data/0.bin").unwrap(), before);
    let report = workspace
        .restore(&request(&workspace, initial, None, Expected::Latest))
        .unwrap();
    assert!(report.failed.is_none());
    assert_eq!(fixture.read("untouched.txt").unwrap(), b"keep this");
    assert_eq!(
        fixture.read("data/0.bin").unwrap(),
        before,
        "the baseline remains available above the retention target"
    );
    drop(workspace);
    let reopened = fixture.open();
    assert!(reopened.selective().is_none());
    assert!(index(&reopened).current("untouched.txt").is_some());
}

#[test]
fn a_removed_unknown_parent_still_updates_its_protected_descendants() {
    let fixture = Fixture::new();
    let workspace = fixture.open();
    workspace.limit_scope("fixture with unknown paths").unwrap();
    let mut mutation = workspace.mutation("model", "write", &fixture.path("src/file.rs"));
    mutation.prepare(b"new source");
    fixture.write("src/file.rs", b"new source");
    assert!(matches!(
        mutation.finish(true),
        Protection::Protected { .. }
    ));
    workspace.flush("external").unwrap();
    assert!(index(&workspace).current("src").is_none());
    fs::remove_file(fixture.path("src/file.rs")).unwrap();
    fs::remove_dir(fixture.path("src")).unwrap();
    workspace.signals.update(|health| {
        health.dirty.insert("src".into());
    });
    workspace.flush("external").unwrap();
    assert_eq!(
        index(&workspace).current_state("src/file.rs"),
        State::Absent
    );
}

#[test]
fn explicit_verify_can_regain_full_coverage_after_the_scope_shrinks() {
    let fixture = Fixture::new();
    let workspace = fixture.open();
    workspace
        .limit_scope("fixture with incomplete coverage")
        .unwrap();
    assert!(workspace.selective().is_some());
    fixture.write("small.rs", b"small");
    let summary = workspace.verify().unwrap();
    assert!(summary.limited.is_none());
    assert!(workspace.selective().is_none());
    assert!(index(&workspace).current("small.rs").is_some());
}

#[test]
fn expanding_scope_rules_retries_discovery_at_the_next_point() {
    let fixture = Fixture::new();
    fixture.write("target/new.rs", b"excluded before policy change");
    let workspace = fixture.open();
    assert!(index(&workspace).current("target/new.rs").is_none());
    let mut policy = Policy::read(&workspace.archive).unwrap().unwrap();
    policy.include.push("target".into());
    policy.save(&workspace.archive).unwrap();
    // Another process adopts the edited policy, as on a live workspace.
    drop(crate::checkpoints::store::Store::open(workspace.store().workspace.clone()).unwrap());
    workspace
        .mark("turn", "narrower", "model", false, false)
        .unwrap();
    assert!(workspace.selective().is_none());
    assert!(index(&workspace).current("target/new.rs").is_some());
}

#[test]
fn another_process_selective_coverage_reaches_requests_and_commands() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"before");
    let session =
        crate::checkpoints::session::Session::offline(fixture.home.path(), fixture.root.path());
    session.begin("model", None);
    let workspace = session.workspace().unwrap();
    assert!(workspace.selective().is_none());
    let mut other =
        crate::checkpoints::store::Store::open(workspace.store().workspace.clone()).unwrap();
    crate::checkpoints::scan::coverage(
        &mut other,
        false,
        "selective: another process reached capacity",
    )
    .unwrap();
    session.notices();
    assert_eq!(
        workspace.selective().as_deref(),
        Some("another process reached capacity")
    );
    let command = workspace.command("model", "read-only");
    assert!(matches!(command.freshness, Freshness::Unavailable(_)));
    command.finish();
    crate::checkpoints::scan::coverage(&mut other, true, "scan").unwrap();
    session.notices();
    assert!(workspace.selective().is_none());
}

#[test]
fn a_new_policy_never_inherits_complete_coverage_of_the_old_scope() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"known");
    fixture.write("target/previous.txt", b"previously excluded");
    let workspace = fixture.open();
    let location = workspace.store().workspace.clone();
    let mut other = crate::checkpoints::store::Store::open(location.clone()).unwrap();
    crate::checkpoints::scan::coverage(&mut other, true, "scan").unwrap();
    let old = other.index.latest();
    let mut policy = other.policy.clone();
    policy.include.push("target".into());
    policy.save(&workspace.archive).unwrap();
    drop(other);
    let other = crate::checkpoints::store::Store::open(location).unwrap();
    let changed_scope = other.index.latest();
    assert!(other.index.coverage_at(old));
    assert!(!other.index.coverage_complete());
    assert!(other.index.current_coverage().is_none());
    workspace.store().refresh().unwrap();
    let path = fixture.path("target/previous.txt");
    let mut mutation = workspace.mutation("model", "write", &path);
    mutation.prepare(b"changed");
    fixture.write("target/previous.txt", b"changed");
    assert!(matches!(
        mutation.finish(true),
        Protection::Protected { .. }
    ));
    let selected = paths(&["target/previous.txt"]);
    let report = workspace
        .restore(&request(
            &workspace,
            changed_scope,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();
    assert!(report.applied.is_empty());
    assert_eq!(
        fixture.read("target/previous.txt").unwrap(),
        b"changed",
        "unknown previous scope must not become an inferred deletion"
    );
}
