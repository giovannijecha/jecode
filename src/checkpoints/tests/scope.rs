//! Scope rules edited in `policy.json`, by this or another process.
use super::*;

#[test]
fn polling_points_keep_complete_observations_without_claiming_live_coverage() {
    let fixture = Fixture::new();
    fixture.write("source.rs", b"source\n");
    let workspace = fixture.open();
    for _ in 0..4 {
        let point = workspace
            .mark("turn", "observed", "test", false, false)
            .unwrap();
        let index = index(&workspace);
        assert!(index.coverage_at(point));
        assert!(!index.coverage_complete());
        assert_eq!(index.current_coverage().unwrap().1, "not watched");
    }
}

#[test]
fn polling_keeps_the_reason_for_an_incomplete_observation() {
    let fixture = Fixture::new();
    let workspace = fixture.open();
    crate::checkpoints::scan::coverage(&mut workspace.store(), true, "previous").unwrap();
    let summary = crate::checkpoints::scan::Summary {
        unreadable: 1,
        ..Default::default()
    };
    let point = workspace
        .covered_point(&summary, Some(("turn", "incomplete", "test", false)))
        .unwrap()
        .unwrap();
    let index = index(&workspace);
    assert!(!index.coverage_at(point));
    let (complete, reason) = index.current_coverage().unwrap();
    assert!(!complete);
    assert!(reason.contains("1 unreadable directories"), "{reason}");
}
use crate::checkpoints::policy::Policy;
use crate::checkpoints::store::Store;

fn edit(workspace: &Workspace, change: impl FnOnce(&mut Policy)) -> Policy {
    let mut policy = Policy::read(&workspace.archive).unwrap().unwrap();
    change(&mut policy);
    policy.save(&workspace.archive).unwrap();
    policy
}

#[test]
fn edited_scope_rules_apply_on_reopen_without_deleting_history() {
    let fixture = Fixture::new();
    fixture.write("keep.txt", b"keep\n");
    fixture.write("notes.log", b"log\n");
    fixture.write("node_modules/pkg/index.js", b"dependency\n");
    let workspace = fixture.open();
    let start = index(&workspace)
        .points
        .values()
        .find(|point| point.kind == "baseline")
        .unwrap()
        .seq;

    // The user edits the rules while no process has the archive open.
    let policy = edit(&workspace, |policy| {
        policy.exclude.push("*.log".into());
        policy.include.push("node_modules".into());
    });
    drop(workspace);
    let reopened = fixture.open();
    assert_eq!(reopened.store().policy, policy);
    let recorded = index(&reopened);
    assert!(
        recorded.current("node_modules/pkg/index.js").is_some(),
        "newly included paths are captured"
    );
    assert!(
        recorded
            .events
            .iter()
            .any(|event| event.text.contains("+exclude *.log")
                && event.text.contains("+include node_modules")),
        "{:?}",
        recorded.events
    );

    // A now excluded file is neither deleted by a full restore nor lost.
    fixture.write("keep.txt", b"changed\n");
    let report = reopened
        .restore(&request(&reopened, start, None, Expected::Latest))
        .unwrap();
    assert!(report.failed.is_none() && report.conflicts.is_empty());
    assert_eq!(fixture.read("keep.txt").unwrap(), b"keep\n");
    assert_eq!(fixture.read("notes.log").unwrap(), b"log\n");
    assert!(recorded.current("notes.log").is_some());
}

#[test]
fn scope_rules_edited_by_another_process_apply_at_the_next_point() {
    let fixture = Fixture::new();
    fixture.write("node_modules/pkg/index.js", b"dependency\n");
    let workspace = fixture.open();
    assert!(
        index(&workspace)
            .current("node_modules/pkg/index.js")
            .is_none()
    );

    // Another process adopts the edited file while this one keeps running.
    let policy = edit(&workspace, |policy| {
        policy.include.push("node_modules".into())
    });
    let location = workspace.store().workspace.clone();
    drop(Store::open(location).unwrap());
    workspace
        .mark("turn", "after", "test", false, true)
        .unwrap();
    assert_eq!(workspace.store().policy, policy);
    assert!(
        index(&workspace)
            .current("node_modules/pkg/index.js")
            .is_none()
    );
    workspace.verify().unwrap();
    assert!(
        index(&workspace)
            .current("node_modules/pkg/index.js")
            .is_some()
    );
}
