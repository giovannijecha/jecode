use super::recovery_tests::create;
use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    collaboration::{Spec, workspace::Workspace},
    sessions::Store,
    test_support::{Directory, HttpFixture, completion, link_directory},
};
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Condvar, Mutex},
    thread,
    time::Instant,
};

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn project() -> (Directory, PathBuf) {
    let fixture = Directory::new();
    let root = fixture.path().join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("file.txt"), "source\n").unwrap();
    (fixture, root)
}

fn storage(home: &Directory, project: &Path, session: &str) -> PathBuf {
    Store::new(home.path().to_path_buf(), project)
        .unwrap()
        .bucket()
        .join("collaboration")
        .join(session)
}

fn wait(agent: &Agent, id: &str) {
    agent
        .collaboration()
        .wait(
            Some(id),
            Duration::from_secs(5),
            &crate::cancel::Cancellation::default(),
        )
        .unwrap();
}

#[test]
fn copy_prepared_before_a_crash_is_reclaimed_on_resume_but_not_while_its_session_lives() {
    let (_temp, project) = project();
    let home = Directory::new();
    let signal = Directory::new();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            "agent::collaboration::reclaim_tests::prepare_then_stop",
            "--nocapture",
        ])
        .env("JECODE_RECLAIM_HOME", home.path())
        .env("JECODE_RECLAIM_PROJECT", &project)
        .env("JECODE_RECLAIM_SIGNAL", signal.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = Running(command.spawn().unwrap());
    let prepared = signal.path().join("prepared");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !prepared.exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "fixture exited before preparing its copy"
        );
        assert!(
            Instant::now() < deadline,
            "fixture did not prepare its copy"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let crashed = fs::read_to_string(&prepared).unwrap();
    let live_copy = storage(&home, &project, &crashed).join("a1");
    assert!(live_copy.join("tree/file.txt").is_file());
    assert_eq!(
        fs::read_to_string(live_copy.join("base/file.txt")).unwrap(),
        "source\n"
    );
    assert!(live_copy.join("baseline.json").is_file());

    // Another conversation in the same project, with its own leftover setup.
    let fixture = HttpFixture::new(vec![
        (200, completion("Seeded", vec![])),
        (200, completion("Checked", vec![])),
    ]);
    let mut other = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&project).unwrap(),
    );
    other.enable_sessions(home.path()).unwrap();
    other.run_turn("Seed", &mut |_| Ok(())).unwrap();
    let other_id = other.sessions().unwrap().id();
    let leftover = storage(&home, &project, &other_id);
    Workspace::prepare(&project, &leftover, "a1", Mode::Isolated).unwrap();
    other.save_session().unwrap();
    drop(other);
    assert!(leftover.join("a1/tree/file.txt").is_file());

    let mut resumed = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&project).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    // The stopped setup's conversation is still live: its lease keeps others out.
    assert!(
        resumed
            .resume(&crashed)
            .unwrap_err()
            .contains("already open")
    );
    let status = resumed.resume(&other_id).unwrap();
    assert!(status.contains("Removed 1 isolated agent copy"), "{status}");
    assert!(!leftover.join("a1").exists());
    assert!(live_copy.join("tree/file.txt").is_file());
    assert!(live_copy.join("baseline.json").is_file());

    drop(child);
    let status = resumed.resume(&crashed).unwrap();
    assert!(status.contains("Removed 1 isolated agent copy"), "{status}");
    assert!(!live_copy.exists());
    // The unused number prepares a fresh copy rather than colliding.
    resumed.agent_tool(&create("isolated")).unwrap();
    wait(&resumed, "a1");
    resumed
        .agent_tool(&super::recovery_tests::action("close"))
        .unwrap();
    assert!(!live_copy.exists());
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
#[ignore = "owned subprocess fixture; launched by the reclaim crash test"]
fn prepare_then_stop() {
    let (Some(home), Some(project), Some(signal)) = (
        std::env::var_os("JECODE_RECLAIM_HOME"),
        std::env::var_os("JECODE_RECLAIM_PROJECT"),
        std::env::var_os("JECODE_RECLAIM_SIGNAL"),
    ) else {
        return;
    };
    let (home, project, signal) = (
        PathBuf::from(home),
        PathBuf::from(project),
        PathBuf::from(signal),
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/test-fixtures")
        .canonicalize()
        .unwrap();
    for path in [&home, &project, &signal] {
        assert!(path.canonicalize().unwrap().starts_with(&root));
    }
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(&project).unwrap(),
    );
    agent.enable_sessions(&home).unwrap();
    agent.prepare_turn("Delegate the inspection").unwrap();
    agent.save_session().unwrap();
    let session = agent.sessions().unwrap().id();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let _ = agent.collaboration().create(
        &project,
        Spec {
            title: "Inspect".into(),
            instructions: "Report".into(),
            context: String::new(),
            mode: Mode::Isolated,
        },
        |_, _| {
            // The copy is prepared; stop before the `created` receipt is saved.
            let staged = signal.join("prepared.tmp");
            fs::write(&staged, &session).unwrap();
            fs::rename(&staged, signal.join("prepared")).unwrap();
            loop {
                thread::sleep(Duration::from_secs(1));
            }
        },
        gate,
    );
}

#[test]
fn resume_keeps_owned_and_foreign_copies_and_reclaims_setup_leftovers() {
    let (_temp, project) = project();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("Completed report", vec![])),
        (200, completion("Next report", vec![])),
    ]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&project).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    parent.agent_tool(&create("isolated")).unwrap();
    wait(&parent, "a1");
    let id = parent.sessions().unwrap().id();
    let storage = storage(&home, &project, &id);
    // Setups that stopped after preparing, while preparing and before
    // writing the ownership marker.
    Workspace::prepare(&project, &storage, "a2", Mode::Isolated).unwrap();
    Workspace::prepare(&project, &storage, "a3", Mode::Isolated).unwrap();
    let marker = storage.join("a3/baseline.json");
    let text = fs::read_to_string(&marker).unwrap();
    fs::write(&marker, text.replace("\"ready\":true", "\"ready\":false")).unwrap();
    fs::create_dir(storage.join("a4")).unwrap();
    // A marker naming another assignment does not prove this copy is ours.
    Workspace::prepare(&project, &storage, "a5", Mode::Isolated).unwrap();
    let marker = storage.join("a5/baseline.json");
    let text = fs::read_to_string(&marker).unwrap();
    fs::write(&marker, text.replace("\"id\":\"a5\"", "\"id\":\"a9\"")).unwrap();
    parent.save_session().unwrap();
    drop(parent);

    let mut resumed = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&project).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    let status = resumed.resume(&id).unwrap();
    assert!(
        status.contains("Removed 3 isolated agent copies"),
        "{status}"
    );
    assert!(status.contains("Kept isolated agent copy"), "{status}");
    assert!(storage.join("a1/tree/file.txt").is_file());
    for name in ["a2", "a3", "a4"] {
        assert!(!storage.join(name).exists(), "{name} remains");
    }
    assert!(storage.join("a5/tree/file.txt").is_file());
    let records = resumed.collaboration().snapshot();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].get("id").and_then(Value::as_str), Some("a1"));
    // New work does not collide with the copy left in place.
    let created = resumed.agent_tool(&create("isolated")).unwrap();
    assert_eq!(
        created.get("created").and_then(Value::as_array).unwrap()[0]
            .get("id")
            .and_then(Value::as_str),
        Some("a6")
    );
    wait(&resumed, "a6");
    for id in ["a1", "a6"] {
        resumed
            .agent_tool(&Value::object([
                ("action", Value::string("close")),
                ("id", Value::string(id)),
            ]))
            .unwrap();
    }
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn reclaiming_a_copy_removes_its_links_without_following_them() {
    let (temp, project) = project();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("canary.txt"), "keep\n").unwrap();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Seeded", vec![]))]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&project).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    parent.run_turn("Seed", &mut |_| Ok(())).unwrap();
    let id = parent.sessions().unwrap().id();
    let storage = storage(&home, &project, &id);
    let copy = Workspace::prepare(&project, &storage, "a1", Mode::Isolated).unwrap();
    let nested = copy.root().join("nested");
    fs::create_dir(&nested).unwrap();
    link_directory(&project, &copy.root().join("project-link"));
    link_directory(&outside, &nested.join("outside-link"));
    parent.save_session().unwrap();
    drop(parent);

    let mut resumed = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&project).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    let status = resumed.resume(&id).unwrap();
    assert!(status.contains("Removed 1 isolated agent copy"), "{status}");
    assert!(!storage.join("a1").exists());
    assert_eq!(
        fs::read_to_string(outside.join("canary.txt")).unwrap(),
        "keep\n"
    );
    assert_eq!(
        fs::read_to_string(project.join("file.txt")).unwrap(),
        "source\n"
    );
    assert!(!storage.join("a1").exists());
    assert_eq!(fixture.finish().len(), 1);
}
