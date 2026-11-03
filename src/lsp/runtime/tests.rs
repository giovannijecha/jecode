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
