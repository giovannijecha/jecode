//! Large scopes protect actual work without inventing an absent baseline.
use super::*;
use crate::checkpoints::inventory::{self, Limits};
use crate::checkpoints::mutation::{Freshness, Protection};
use crate::checkpoints::policy::Policy;
use crate::checkpoints::record::State;

fn small_budget(fixture: &Fixture) {
    let workspace = fixture.open();
    workspace
        .configure(Some(1 << 20), Some(512 << 10), "test")
        .unwrap();
}

fn heavy(fixture: &Fixture) {
    for index in 0..4 {
        fixture.write(&format!("data/{index}.bin"), &noise(index, 128 << 10));
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
fn oversized_baseline_preserves_history_and_protects_touched_paths() {
    let fixture = Fixture::new();
    small_budget(&fixture);
    heavy(&fixture);
    fixture.write("untouched.txt", b"keep this");
    let workspace = fixture.open();
    assert_eq!(workspace.readiness(), workspace::Readiness::Ready);
    assert!(workspace.selective().is_some());
    assert!(
        index(&workspace).current("data/0.bin").is_none(),
        "no arbitrary prefix was copied"
    );
    assert!(index(&workspace).current("untouched.txt").is_none());
    let initial = workspace
        .mark("turn", "partial", "model", false, false)
        .unwrap();
    let before = fixture.read("data/0.bin").unwrap();
    let mut mutation = workspace.mutation("model", "write", &fixture.path("data/0.bin"));
    mutation.prepare(b"changed");
    fixture.write("data/0.bin", b"changed");
    let Protection::Protected { before: saved, .. } = mutation.finish(true) else {
        panic!("touched file was not protected");
    };
    assert!(matches!(
        workspace.command("model", "read-only").freshness,
        Freshness::Selective(_)
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
        "unknown history is never treated as absence"
    );
    drop(workspace);
    let reopened = fixture.open();
    assert!(
        reopened.selective().is_some(),
        "coverage survives process-local reopen"
    );
}

#[test]
fn a_removed_unknown_parent_still_updates_its_protected_descendants() {
    let fixture = Fixture::new();
    small_budget(&fixture);
    heavy(&fixture);
    let workspace = fixture.open();
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
    small_budget(&fixture);
    heavy(&fixture);
    let workspace = fixture.open();
    assert!(workspace.selective().is_some());
    fs::remove_dir_all(fixture.path("data")).unwrap();
    fixture.write("small.rs", b"small");
    let summary = workspace.verify().unwrap();
    assert!(summary.limited.is_none());
    assert!(workspace.selective().is_none());
    assert!(index(&workspace).current("small.rs").is_some());
}

#[test]
fn narrowing_scope_rules_retries_discovery_at_the_next_point() {
    let fixture = Fixture::new();
    small_budget(&fixture);
    heavy(&fixture);
    fixture.write("small.rs", b"small");
    let workspace = fixture.open();
    assert!(workspace.selective().is_some());
    let mut policy = Policy::read(&workspace.archive).unwrap().unwrap();
    policy.exclude.push("data".into());
    policy.save(&workspace.archive).unwrap();
    // Another process adopts the edited policy, as on a live workspace.
    drop(crate::checkpoints::store::Store::open(workspace.store().workspace.clone()).unwrap());
    workspace
        .mark("turn", "narrower", "model", false, false)
        .unwrap();
    assert!(workspace.selective().is_none());
    assert!(index(&workspace).current("small.rs").is_some());
    assert!(index(&workspace).current("data/0.bin").is_none());
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
    assert!(matches!(command.freshness, Freshness::Selective(_)));
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
