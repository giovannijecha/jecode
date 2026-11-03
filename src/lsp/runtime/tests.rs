use super::*;
use crate::lsp::{Store, fixture::Fixture};
use std::fs;

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
