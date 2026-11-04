use super::*;
use crate::lsp::{Store, fixture::Fixture};
use std::fs;
use std::thread;

fn definition(path: PathBuf, column: u32) -> Request {
    Request::Definition {
        path,
        line: 1,
        column,
        server: None,
    }
}

fn run(runtime: &Runtime, fixture: &Fixture, path: PathBuf) -> Outcome {
    runtime.execute(
        fixture.workspace(),
        &[],
        definition(path, 1),
        &Cancellation::default(),
    )
}

fn wait_for(fixture: &Fixture, text: &str, failure: &str) {
    // Shutdown includes the handshake timeouts; a loaded parallel run needs
    // headroom beyond their nominal 1.2 s.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !fs::read_to_string(fixture.marker())
        .unwrap_or_default()
        .contains(text)
    {
        assert!(Instant::now() < deadline, "{failure}");
        thread::sleep(Duration::from_millis(25));
    }
}

fn registered(runtime: &Runtime) -> Arc<Entry> {
    let entries = runtime.0.entries.lock().unwrap();
    assert_eq!(entries.len(), 1);
    Arc::clone(entries.values().next().unwrap())
}

fn observed(entry: &Entry) -> (String, Option<u32>) {
    let state = entry.state.lock().unwrap();
    (state.status.clone(), state.client.as_ref().map(Client::pid))
}

#[test]
fn off_then_on_without_a_request_starts_a_fresh_instance() {
    let fixture = Fixture::new();
    let home = fixture.workspace().join("home");
    let store = Store::new(home.clone());
    let profile = fixture.profile();
    store.save(profile.clone()).unwrap();
    let runtime = Runtime::with_home(home);
    let first = run(&runtime, &fixture, fixture.sample());
    assert_eq!(first.status, "ready", "{}", first.detail);
    store.set_enabled(&profile.id, false).unwrap();
    wait_for(&fixture, "exit", "Off did not stop the idle instance");
    assert!(runtime.statuses().is_empty());
    store.set_enabled(&profile.id, true).unwrap();
    let again = run(&runtime, &fixture, fixture.sample());
    assert_eq!(again.status, "ready", "{}", again.detail);
    runtime.shutdown();
    drop(runtime);
    assert!(
        fs::read_to_string(fixture.marker())
            .unwrap()
            .contains("exit"),
        "The last runtime owner did not wait for its instances to stop"
    );
}

#[test]
fn entry_replaces_stopping_or_differently_configured_instances() {
    let runtime = Runtime::new();
    let profile = Profile::draft_guided_rust();
    let root = Path::new("root");
    let first = runtime.entry(&profile, root, "test");
    assert!(Arc::ptr_eq(&first, &runtime.entry(&profile, root, "test")));
    let mut changed = profile.clone();
    changed.arguments.push("--changed".into());
    let second = runtime.entry(&changed, root, "test");
    assert!(!Arc::ptr_eq(&first, &second));
    assert!(first.stop.requested());
    second.stop.cancel();
    let third = runtime.entry(&changed, root, "test");
    assert!(!Arc::ptr_eq(&second, &third));
    assert!(!third.stop.requested());
    runtime.retire(&second);
    assert!(Arc::ptr_eq(&third, &registered(&runtime)));
    runtime.retire(&third);
    assert!(runtime.statuses().is_empty());
}

#[test]
fn turn_off_returns_while_a_request_is_in_flight() {
    let fixture = Fixture::new();
    let home = fixture.workspace().join("home");
    let profile = fixture.profile();
    Store::new(home.clone()).save(profile.clone()).unwrap();
    let runtime = Runtime::with_home(home);
    assert_eq!(run(&runtime, &fixture, fixture.sample()).status, "ready");
    let pid = runtime.statuses()[0].pid.expect("server pid");
    assert!(fixture.running(pid));
    let stuck_runtime = runtime.clone();
    let stuck_root = fixture.workspace().to_path_buf();
    let stuck_path = fixture.sample();
    // The fixture never answers this request, so it stays in flight throughout.
    let stuck = thread::spawn(move || {
        stuck_runtime.execute(
            &stuck_root,
            &[],
            definition(stuck_path, 9),
            &Cancellation::default(),
        )
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while fs::read_to_string(fixture.marker())
        .unwrap()
        .matches("textDocument/definition")
        .count()
        < 2
    {
        assert!(Instant::now() < deadline, "Stuck call was not dispatched");
        thread::sleep(Duration::from_millis(10));
    }
    let started = Instant::now();
    runtime.configuration_changed(&profile.id);
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "Turn off waited for the instance: {:?}",
        started.elapsed()
    );
    assert!(runtime.statuses().is_empty());
    assert_eq!(stuck.join().unwrap().status, "settings_changed");
    // Unanswered, the shutdown handshake times out and the server is killed.
    let deadline = Instant::now() + Duration::from_secs(10);
    while fixture.running(pid) {
        assert!(
            Instant::now() < deadline,
            "Turn off did not stop the instance"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn local_document_failures_keep_the_server_and_success_clears_failures() {
    let fixture = Fixture::new();
    let home = fixture.workspace().join("home");
    Store::new(home.clone()).save(fixture.profile()).unwrap();
    let runtime = Runtime::with_home(home);
    assert_eq!(run(&runtime, &fixture, fixture.sample()).status, "ready");
    let entry = registered(&runtime);
    let (_, pid) = observed(&entry);
    assert!(pid.is_some());
    entry.state.lock().unwrap().failures = 2;
    assert_eq!(run(&runtime, &fixture, fixture.sample()).status, "ready");
    assert_eq!(entry.state.lock().unwrap().failures, 0);

    // The idle pump polls the open document while it is not readable text.
    fs::write(fixture.sample(), [0xff, 0xfe, b'\n']).unwrap();
    thread::sleep(Duration::from_millis(1200));
    assert_eq!(observed(&entry), ("ready".into(), pid));
    fs::write(fixture.sample(), "pub fn sample() {}\n").unwrap();

    let large = fixture.workspace().join("large.rs");
    fs::write(&large, "a".repeat(8 * 1024 * 1024 - 64)).unwrap();
    for _ in 0..3 {
        let outcome = run(&runtime, &fixture, large.clone());
        assert_eq!(outcome.status, "unavailable", "{}", outcome.detail);
        assert!(outcome.detail.contains("too large"), "{}", outcome.detail);
    }
    assert_eq!(observed(&entry), ("ready".into(), pid));
    assert_eq!(run(&runtime, &fixture, fixture.sample()).status, "ready");
    assert_eq!(observed(&entry), ("ready".into(), pid));
    runtime.shutdown();
}

#[test]
fn only_requests_report_an_instance_busy() {
    let fixture = Fixture::new();
    let home = fixture.workspace().join("home");
    Store::new(home.clone()).save(fixture.profile()).unwrap();
    let runtime = Runtime::with_home(home);
    assert_eq!(run(&runtime, &fixture, fixture.sample()).status, "ready");
    let entry = registered(&runtime);
    let (_, pid) = observed(&entry);
    // Holding the state stands in for an idle pump tick.
    let held = entry.state.lock().unwrap();
    let status = runtime.statuses().remove(0);
    assert_eq!((status.state.as_str(), status.pid), ("ready", pid));
    let queued_runtime = runtime.clone();
    let workspace = fixture.workspace().to_path_buf();
    let path = fixture.sample();
    let queued = thread::spawn(move || {
        queued_runtime.execute(
            &workspace,
            &[],
            definition(path, 1),
            &Cancellation::default(),
        )
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while entry.requests.load(Ordering::Acquire) == 0 {
        assert!(Instant::now() < deadline, "Request was not queued");
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(runtime.statuses()[0].state, "busy");
    drop(held);
    assert_eq!(queued.join().unwrap().status, "ready");
    let status = runtime.statuses().remove(0);
    assert_eq!((status.state.as_str(), status.pid), ("ready", pid));
    runtime.shutdown();
}

#[test]
fn relative_executable_resolves_from_the_working_folder_for_runtime_and_check() {
    let fixture = Fixture::new();
    let nested = fixture.workspace().join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("Cargo.toml"), "").unwrap();
    let source = nested.join("lib.rs");
    fs::write(&source, "pub fn nested() {}\n").unwrap();
    let mut profile = fixture.profile();
    profile.executable = format!("./server{}", std::env::consts::EXE_SUFFIX);
    profile.root_override = None;
    let home = fixture.workspace().join("home");
    Store::new(home.clone()).save(profile.clone()).unwrap();
    let runtime = Runtime::with_home(home);
    let outcome = run(&runtime, &fixture, source);
    assert_eq!(outcome.status, "ready", "{}", outcome.detail);
    assert_eq!(outcome.root, Some(fs::canonicalize(&nested).unwrap()));
    runtime.shutdown();
    profile.root_override = Some(nested);
    let observation = check(&profile, fixture.workspace(), &Cancellation::default()).unwrap();
    assert!(Path::new(&observation.executable).is_file());
}

#[test]
fn a_document_lost_during_a_request_invalidates_its_result_without_a_crash() {
    let fixture = Fixture::new();
    let home = fixture.workspace().join("home");
    Store::new(home.clone()).save(fixture.profile()).unwrap();
    let runtime = Runtime::with_home(home);
    for replacement in [None, Some(vec![0xff, 0xfe])] {
        fs::write(fixture.sample(), "pub fn sample() {}\n").unwrap();
        assert_eq!(run(&runtime, &fixture, fixture.sample()).status, "ready");
        let pid = runtime.statuses()[0].pid;
        let expected = fs::read_to_string(fixture.marker())
            .unwrap()
            .matches("textDocument/definition")
            .count()
            + 1;
        let worker_runtime = runtime.clone();
        let workspace = fixture.workspace().to_path_buf();
        let path = fixture.sample();
        let request = thread::spawn(move || {
            worker_runtime.execute(
                &workspace,
                &[],
                definition(path, 6),
                &Cancellation::default(),
            )
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while fs::read_to_string(fixture.marker())
            .unwrap()
            .matches("textDocument/definition")
            .count()
            < expected
        {
            assert!(Instant::now() < deadline, "Slow call was not dispatched");
            thread::sleep(Duration::from_millis(10));
        }
        if let Some(bytes) = replacement {
            fs::write(fixture.sample(), bytes).unwrap();
        } else {
            fs::remove_file(fixture.sample()).unwrap();
        }
        let result = request.join().unwrap();
        assert_eq!(result.status, "ready", "{}", result.detail);
        assert_eq!(result.freshness, "stale", "{}", result.detail);
        assert_eq!(runtime.statuses()[0].pid, pid);
    }
    runtime.shutdown();
}
