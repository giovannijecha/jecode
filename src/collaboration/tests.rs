use super::*;
use crate::{
    agent::Agent,
    backend::openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion},
    tools::Tools,
};
use std::time::Duration;

fn repository() -> (Directory, std::path::PathBuf) {
    let temp = Directory::new();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&project)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    std::fs::write(project.join("file.txt"), "source\n").unwrap();
    git(&["add", "file.txt"]);
    git(&[
        "-c",
        "user.name=Giovanni Jecha",
        "-c",
        "user.email=100841799+giovannijecha@users.noreply.github.com",
        "commit",
        "-qm",
        "initial",
    ]);
    (temp, project)
}

#[test]
fn visible_updates_and_recovered_snapshot_are_bounded() {
    let project = Directory::new();
    let storage = Directory::new();
    let workspace = Workspace::prepare(
        project.path(),
        storage.path(),
        "a1",
        workspace::Mode::Shared,
    )
    .unwrap();
    let seed = Value::object([
        ("id", Value::string("a1")),
        ("number", Value::number(1)),
        ("state", Value::string("returned")),
        ("workspace", Value::string("shared")),
        ("workspace_info", workspace.value()),
        ("directory", Value::string(project.path().to_string_lossy())),
    ]);
    let observer = Observer::default();
    observer
        .0
        .state
        .lock()
        .unwrap()
        .records
        .push(Record::from_value(&seed).unwrap());
    for _ in 0..(MAX_UPDATES + 10) {
        observer.message("a1", &"x".repeat(UPDATE_CHARS + 10));
    }
    let snapshot = &observer.snapshot()[0];
    let updates = snapshot.get("updates").and_then(Value::as_array).unwrap();
    assert_eq!(updates.len(), MAX_UPDATES);
    assert!(
        updates
            .iter()
            .all(|update| update.as_str().unwrap().chars().count() == UPDATE_CHARS)
    );
    assert_eq!(snapshot.get("updates_truncated"), Some(&Value::Bool(true)));
    let recovered = Record::from_value(snapshot).unwrap().value();
    assert_eq!(
        recovered
            .get("updates")
            .and_then(Value::as_array)
            .unwrap()
            .len(),
        MAX_UPDATES
    );
}

#[test]
fn sequential_closure_releases_workers_and_preserves_monotonic_ids_after_resume() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(
        (0..25)
            .map(|_| (200, completion("Checked", vec![])))
            .collect(),
    );
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    let observer = parent.collaboration();

    let run_one = |observer: &Observer, expected: usize| {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let created = observer
            .create(
                project.path(),
                Spec {
                    title: "Check".into(),
                    instructions: "Report".into(),
                    context: String::new(),
                    mode: workspace::Mode::Shared,
                },
                |root| {
                    Ok(Agent::new(
                        OpenRouter::fixture(fixture.endpoint.clone()),
                        Tools::new(root)?,
                    ))
                },
                gate.clone(),
            )
            .unwrap();
        let id = format!("a{expected}");
        assert_eq!(created.get("id").and_then(Value::as_str), Some(id.as_str()));
        observer
            .receipt("created", None, 0, "Agents", vec![id.clone()])
            .unwrap();
        let (lock, ready) = &*gate;
        *lock.lock().unwrap() = true;
        ready.notify_all();
        let records = observer
            .wait(Some(&id), Duration::from_secs(5), &Cancellation::default())
            .unwrap();
        assert_eq!(
            records[0].get("state").and_then(Value::as_str),
            Some("returned")
        );
        assert_eq!(records[0].get("reviewed"), Some(&Value::Bool(false)));
        observer.review(&id, 1, "Checked report".into()).unwrap();
        observer.close(&id).unwrap();
        let state = observer.0.state.lock().unwrap();
        assert!(state.records.is_empty());
        assert!(state.workers.is_empty());
        assert!(!state.receipts.iter().any(|event| {
            event.get("id").and_then(Value::as_str) == Some(id.as_str())
                && event.get("kind").and_then(Value::as_str) == Some("stopped")
        }));
        let final_record = state
            .receipts
            .last()
            .unwrap()
            .get("records")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(final_record.len(), 1);
        assert_eq!(
            final_record[0].get("workspace_closed"),
            Some(&Value::Bool(true))
        );
    };

    for number in 1..=24 {
        run_one(&observer, number);
    }
    let session = parent.sessions().unwrap().id();
    parent.save_session().unwrap();
    drop(observer);
    drop(parent);

    let mut resumed = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&session).unwrap();
    assert!(resumed.collaboration().snapshot().is_empty());
    run_one(&resumed.collaboration(), 25);
    assert_eq!(fixture.finish().len(), 25);
}

#[test]
fn failed_closed_receipt_keeps_record_visible_for_retry() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    let observer = parent.collaboration();
    let closed = Value::object([
        ("id", Value::string("a1")),
        ("number", Value::number(1)),
        ("state", Value::string("stopped")),
        ("workspace_closed", Value::Bool(true)),
        ("received_version", Value::number(3)),
    ]);
    observer
        .0
        .state
        .lock()
        .unwrap()
        .records
        .push(Record::from_value(&closed).unwrap());
    let parent_binding = observer.0.parent.lock().unwrap().take();
    assert_eq!(
        observer.close("a1").unwrap_err(),
        "Session storage is unavailable"
    );
    assert_eq!(observer.snapshot().len(), 1);
    assert!(observer.events_since(0).1.is_empty());
    *observer.0.parent.lock().unwrap() = parent_binding;
    assert_eq!(
        observer.close("a1").unwrap().get("closed"),
        Some(&Value::Bool(true))
    );
    assert!(observer.snapshot().is_empty());
    assert_eq!(
        observer.events_since(0).1[0].get("version"),
        Some(&Value::number(3))
    );
    assert_eq!(fixture.finish().len(), 0);
}

#[test]
fn removed_isolated_workspace_can_be_closed_after_failed_receipt_and_resume() {
    let (_temp, project) = repository();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Checked", vec![]))]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&project).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    let observer = parent.collaboration();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    observer
        .create(
            &project,
            Spec {
                title: "Inspect".into(),
                instructions: "Report".into(),
                context: String::new(),
                mode: workspace::Mode::Isolated,
            },
            |root| {
                Ok(Agent::new(
                    OpenRouter::fixture(fixture.endpoint.clone()),
                    Tools::new(root)?,
                ))
            },
            gate.clone(),
        )
        .unwrap();
    observer
        .receipt("created", None, 0, "Agents", vec!["a1".into()])
        .unwrap();
    let (lock, ready) = &*gate;
    *lock.lock().unwrap() = true;
    ready.notify_all();
    observer
        .wait(Some("a1"), Duration::from_secs(5), &Cancellation::default())
        .unwrap();
    let session = parent.sessions().unwrap().id();
    parent.save_session().unwrap();
    let binding = observer.0.parent.lock().unwrap().take();
    assert_eq!(
        observer.close("a1").unwrap_err(),
        "Session storage is unavailable"
    );
    assert_eq!(
        observer.snapshot()[0].get("workspace_closed"),
        Some(&Value::Bool(true))
    );
    drop(binding);
    drop(observer);
    drop(parent);

    let mut resumed = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&project).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&session).unwrap();
    let observer = resumed.collaboration();
    let record = observer.snapshot().remove(0);
    assert_eq!(record.get("state").and_then(Value::as_str), Some("unknown"));
    assert_eq!(record.get("workspace_closed"), Some(&Value::Bool(false)));
    let info = record.get("workspace_info").unwrap().clone();
    let storage = std::path::PathBuf::from(info.get("storage").and_then(Value::as_str).unwrap());
    assert!(!storage.exists());

    std::fs::create_dir(&storage).unwrap();
    assert!(
        observer
            .close("a1")
            .unwrap_err()
            .contains("owned files may remain")
    );
    assert_eq!(observer.snapshot().len(), 1);
    std::fs::remove_dir(&storage).unwrap();
    if let Value::Object(fields) = &mut observer.0.state.lock().unwrap().records[0].workspace_info {
        fields.insert(
            "storage".into(),
            Value::string(project.join("a1").to_string_lossy()),
        );
    }
    assert!(
        observer
            .close("a1")
            .unwrap_err()
            .contains("ownership path is invalid")
    );
    assert_eq!(observer.snapshot().len(), 1);
    observer.0.state.lock().unwrap().records[0].workspace_info = info;
    observer.close("a1").unwrap();
    assert!(observer.snapshot().is_empty());
    assert_eq!(fixture.finish().len(), 1);
}
