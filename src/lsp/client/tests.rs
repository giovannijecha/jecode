use super::*;
use crate::lsp::{Request, Runtime, Store, fixture::Fixture};

fn definition(runtime: &Runtime, fixture: &Fixture, name: &str) -> crate::lsp::Outcome {
    runtime.execute(
        fixture.workspace(),
        &[],
        Request::Definition {
            path: fixture.workspace().join(name),
            line: 1,
            column: 1,
            server: None,
        },
        &Cancellation::default(),
    )
}

/// Files exist before startup: a new source file later is a dependency change.
fn started(fixture: &Fixture, files: &[&str]) -> Runtime {
    for name in files {
        std::fs::write(fixture.workspace().join(name), "pub fn x() {}\n").unwrap();
    }
    let home = fixture.workspace().join("home");
    Store::new(home.clone()).save(fixture.profile()).unwrap();
    Runtime::with_home(home)
}

#[test]
fn undrained_notification_burst_keeps_the_same_server() {
    let fixture = Fixture::new();
    let runtime = started(&fixture, &["burst.rs"]);
    let burst = definition(&runtime, &fixture, "burst.rs");
    assert_eq!(burst.status, "ready", "{}", burst.detail);
    let pid = runtime.statuses()[0].pid;
    // The fixture sends 1,000 progress notifications after its reply.
    std::thread::sleep(Duration::from_millis(300));
    let next = definition(&runtime, &fixture, "sample.rs");
    assert_eq!(next.status, "ready", "{}", next.detail);
    assert_eq!(runtime.statuses()[0].pid, pid);
    runtime.shutdown();
}

#[test]
fn capabilities_do_not_request_work_done_progress() {
    assert!(client_capabilities().get("window").is_none());
}
