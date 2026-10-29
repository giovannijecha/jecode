use super::*;
use crate::test_support::Directory;
use std::{hint::black_box, time::Instant};

/// Collaborators `a1`..`aN` in a shared workspace, each with `tools` reads.
fn seeded(count: usize, tools: usize) -> (Observer, Directory, Directory) {
    let project = Directory::new();
    let storage = Directory::new();
    let observer = Observer::default();
    for number in 1..=count {
        let id = format!("a{number}");
        let workspace =
            Workspace::prepare(project.path(), storage.path(), &id, workspace::Mode::Shared)
                .unwrap();
        let seed = Value::object([
            ("id", Value::string(&id)),
            ("number", Value::number(number)),
            ("state", Value::string("returned")),
            ("workspace", Value::string("shared")),
            ("workspace_info", workspace.value()),
            ("directory", Value::string(project.path().to_string_lossy())),
        ]);
        observer
            .0
            .state
            .lock()
            .unwrap()
            .records
            .push(Tracked::new(Record::from_value(&seed).unwrap()));
        for call in 0..tools {
            let call = format!("c{call}");
            let arguments = Value::object([("path", Value::string("src/main.rs"))]);
            observer.tool_started(&id, &call, "read", &arguments);
            let result = Value::object([("content", Value::string("fn main() {}\n".repeat(300)))]);
            observer.tool_finished(&id, &call, "read", "complete", &result);
        }
    }
    (observer, project, storage)
}

#[test]
fn a_snapshot_copies_only_the_records_written_since_it_was_seen() {
    let (observer, _project, _storage) = seeded(2, 1);
    let entries = observer.snapshot_since(&[]).unwrap();
    assert!(entries.iter().all(|(_, value)| value.is_some()));
    let seen: Vec<Seen> = entries.into_iter().map(|(seen, _)| seen).collect();
    assert!(observer.snapshot_since(&seen).is_none(), "nothing changed");

    observer.message("a2", "Looking");
    let entries = observer.snapshot_since(&seen).unwrap();
    assert_eq!(entries[0], (seen[0].clone(), None), "a1 is not copied");
    assert_eq!(entries[1].0.0, "a2");
    assert_ne!(entries[1].0.1, seen[1].1);
    let activity = entries[1].1.as_ref().unwrap().get("activity");
    assert_eq!(activity, Some(&Value::string("Looking")));

    // Another collaboration never matches revisions seen here.
    let (other, _project, _storage) = seeded(2, 0);
    let entries = other.snapshot_since(&seen).unwrap();
    assert!(entries.iter().all(|(_, value)| value.is_some()));
}

#[test]
#[ignore = "synthetic release benchmark; no network or personal configuration"]
fn collaborator_refresh_before_and_after() {
    let redactor = crate::redact::Redactor::new("fixture-secret-value".into());
    let median = |mut times: Vec<f64>| {
        times.sort_by(f64::total_cmp);
        times[times.len() / 2]
    };
    for tools in [100, 400] {
        let (observer, _project, _storage) = seeded(4, tools);
        for write in [false, true] {
            // Before: every tick copied, redacted twice and compared all records.
            let mut previous = Vec::new();
            let mut before = Vec::new();
            for _ in 0..21 {
                if write {
                    observer.message("a4", "Working");
                }
                let started = Instant::now();
                let records: Vec<_> = observer
                    .snapshot()
                    .iter()
                    .map(|record| redactor.value(&redactor.value(record)))
                    .collect();
                if black_box(records != previous) {
                    previous = records;
                }
                before.push(started.elapsed().as_secs_f64() * 1e6);
            }
            // After: only records written since the last tick are copied.
            let mut seen: Vec<Seen> = Vec::new();
            let mut after = Vec::new();
            for _ in 0..21 {
                if write {
                    observer.message("a4", "Working");
                }
                let started = Instant::now();
                if let Some(entries) = observer.snapshot_since(&seen) {
                    seen = entries
                        .into_iter()
                        .map(|(seen, value)| {
                            black_box(value.map(|value| redactor.value(&redactor.value(&value))));
                            seen
                        })
                        .collect();
                }
                after.push(started.elapsed().as_secs_f64() * 1e6);
            }
            let name = if write { "one_working" } else { "idle" };
            println!(
                "SUMMARY,collaborator_tick_{name},4x{tools},before_us={:.1},after_us={:.1}",
                median(before),
                median(after)
            );
        }
    }
}
