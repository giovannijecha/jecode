use super::*;
use crate::cancel::Cancellation;

#[test]
fn owned_server_initializes_navigates_and_pulls_current_empty_diagnostics() {
    let fixture = fixture::Fixture::new();
    let home = fixture.workspace().join("home");
    let store = Store::new(home.clone());
    let profile = fixture.profile();
    store.save(profile.clone()).unwrap();
    let runtime = Runtime::with_home(home);
    let status = runtime.status(fixture.workspace(), &[], None);
    assert_eq!(status.servers[0].state, "not_started");
    assert!(!fixture.marker().exists());
    let result = runtime.execute(
        fixture.workspace(),
        &[],
        Request::Definition {
            path: fixture.sample(),
            line: 1,
            column: 1,
            server: None,
        },
        &Cancellation::default(),
    );
    assert_eq!(result.status, "ready", "{}", result.detail);
    assert_eq!(result.locations.len(), 1);
    let location = std::path::PathBuf::from(
        result.locations[0]
            .get("path")
            .and_then(crate::json::Value::as_str)
            .unwrap(),
    );
    assert_eq!(
        std::fs::canonicalize(location).unwrap(),
        std::fs::canonicalize(fixture.sample()).unwrap()
    );
    assert!(fixture.marker().exists());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !std::fs::read_to_string(fixture.marker())
        .unwrap()
        .contains("registration_reply")
    {
        assert!(
            std::time::Instant::now() < deadline,
            "Idle pump did not answer server requests"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    std::thread::sleep(std::time::Duration::from_millis(600));
    std::fs::write(fixture.workspace().join("watch.txt"), "new").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !std::fs::read_to_string(fixture.marker())
        .unwrap()
        .contains("workspace/didChangeWatchedFiles")
    {
        assert!(
            std::time::Instant::now() < deadline,
            "Idle pump did not report watched file creation"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let pid = runtime.statuses()[0].pid;
    let invalid = runtime.execute(
        fixture.workspace(),
        &[],
        Request::Definition {
            path: fixture.sample(),
            line: 1,
            column: 500,
            server: None,
        },
        &Cancellation::default(),
    );
    assert_eq!(invalid.status, "invalid_input");
    let rejected = runtime.execute(
        fixture.workspace(),
        &[],
        Request::Definition {
            path: fixture.sample(),
            line: 1,
            column: 8,
            server: None,
        },
        &Cancellation::default(),
    );
    assert_eq!(rejected.status, "server_error");
    assert_eq!(runtime.statuses()[0].pid, pid);
    std::fs::write(fixture.sample(), "pub fn changed() {}\n").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !std::fs::read_to_string(fixture.marker())
        .unwrap()
        .contains("textDocument/didChange")
    {
        assert!(
            std::time::Instant::now() < deadline,
            "Idle pump did not send didChange"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let again = runtime.execute(
        fixture.workspace(),
        &[],
        Request::Definition {
            path: fixture.sample(),
            line: 1,
            column: 1,
            server: None,
        },
        &Cancellation::default(),
    );
    assert_eq!(again.status, "ready");
    assert_eq!(runtime.statuses()[0].pid, pid);
    let report = runtime.execute(
        fixture.workspace(),
        &[],
        Request::Diagnostics {
            path: fixture.sample(),
            server: None,
        },
        &Cancellation::default(),
    );
    assert_eq!(report.status, "ready", "{}", report.detail);
    assert_eq!(report.freshness, "current");
    assert_eq!(
        report.diagnostics.len(),
        1,
        "Hybrid push findings must survive empty pull reports"
    );
    std::fs::write(fixture.sample(), "pub fn fixed() {}\n").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let cleared = runtime.execute(
            fixture.workspace(),
            &[],
            Request::Diagnostics {
                path: fixture.sample(),
                server: None,
            },
            &Cancellation::default(),
        );
        if cleared.freshness == "current" && cleared.diagnostics.is_empty() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Versioned push clear did not replace prior findings: {cleared:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    runtime.shutdown();
}

#[test]
fn cancelled_startup_does_not_suppress_restart_and_off_shuts_down() {
    let fixture = fixture::Fixture::new();
    let home = fixture.workspace().join("home");
    let store = Store::new(home.clone());
    let profile = fixture.profile();
    store.save(profile.clone()).unwrap();
    let runtime = Runtime::with_home(home);
    let cancelled = Cancellation::default();
    cancelled.cancel();
    for _ in 0..3 {
        let result = runtime.execute(
            fixture.workspace(),
            &[],
            Request::Definition {
                path: fixture.sample(),
                line: 1,
                column: 1,
                server: None,
            },
            &cancelled,
        );
        assert_eq!(result.status, "cancelled");
    }
    assert!(!fixture.marker().exists());
    let result = runtime.execute(
        fixture.workspace(),
        &[],
        Request::Definition {
            path: fixture.sample(),
            line: 1,
            column: 1,
            server: None,
        },
        &Cancellation::default(),
    );
    assert_eq!(result.status, "ready", "{}", result.detail);
    let pid = runtime.statuses()[0].pid.expect("server pid");
    store.set_enabled(&profile.id, false).unwrap();
    fixture.wait_stopped(pid, "Off did not shut down the owned server");
    let unavailable = runtime.execute(
        fixture.workspace(),
        &[],
        Request::Definition {
            path: fixture.sample(),
            line: 1,
            column: 1,
            server: None,
        },
        &Cancellation::default(),
    );
    assert_eq!(unavailable.status, "off");
    runtime.shutdown();
}

#[test]
fn missing_selected_executable_stays_not_found_without_restart_suppression() {
    let fixture = fixture::Fixture::new();
    let home = fixture.workspace().join("home");
    let store = Store::new(home.clone());
    let mut profile = fixture.profile();
    profile.executable = fixture
        .workspace()
        .join("absent-server")
        .to_string_lossy()
        .into_owned();
    store.save(profile).unwrap();
    let runtime = Runtime::with_home(home);
    for _ in 0..4 {
        let outcome = runtime.execute(
            fixture.workspace(),
            &[],
            Request::Definition {
                path: fixture.sample(),
                line: 1,
                column: 1,
                server: None,
            },
            &Cancellation::default(),
        );
        assert_eq!(outcome.status, "not_found", "{}", outcome.detail);
    }
    assert_eq!(runtime.statuses()[0].state, "not_found");
    assert!(!fixture.marker().exists());
}

#[test]
fn evicted_document_reopens_with_higher_version_and_late_push_is_stale() {
    let fixture = fixture::Fixture::new();
    let home = fixture.workspace().join("home");
    Store::new(home.clone()).save(fixture.profile()).unwrap();
    let runtime = Runtime::with_home(home);
    let first = runtime.execute(
        fixture.workspace(),
        &[],
        Request::Definition {
            path: fixture.sample(),
            line: 1,
            column: 1,
            server: None,
        },
        &Cancellation::default(),
    );
    assert_eq!(first.status, "ready", "{}", first.detail);
    for index in 0..128 {
        let path = fixture.workspace().join(format!("other{index}.rs"));
        std::fs::write(&path, "pub fn x() {}\n").unwrap();
        let result = runtime.execute(
            fixture.workspace(),
            &[],
            Request::Definition {
                path,
                line: 1,
                column: 1,
                server: None,
            },
            &Cancellation::default(),
        );
        assert_eq!(result.status, "ready", "{}", result.detail);
    }
    let reopened = runtime.execute(
        fixture.workspace(),
        &[],
        Request::Definition {
            path: fixture.sample(),
            line: 1,
            column: 1,
            server: None,
        },
        &Cancellation::default(),
    );
    assert_eq!(reopened.status, "ready");
    assert!(reopened.document_version > first.document_version);
    let report = runtime.execute(
        fixture.workspace(),
        &[],
        Request::Diagnostics {
            path: fixture.sample(),
            server: None,
        },
        &Cancellation::default(),
    );
    assert_eq!(report.freshness, "stale");
    assert_eq!(report.diagnostics.len(), 1);
    runtime.shutdown();
}

#[test]
fn one_enabled_profile_wins_over_matching_disabled_profile() {
    let fixture = fixture::Fixture::new();
    let home = fixture.workspace().join("home");
    let store = Store::new(home.clone());
    let enabled = fixture.profile();
    let mut disabled = fixture.profile();
    disabled.enabled = false;
    let disabled_id = disabled.id.clone();
    store.save(enabled.clone()).unwrap();
    store.save(disabled).unwrap();
    let runtime = Runtime::with_home(home);
    let request = || Request::Definition {
        path: fixture.sample(),
        line: 1,
        column: 1,
        server: None,
    };
    let result = runtime.execute(
        fixture.workspace(),
        &[],
        request(),
        &Cancellation::default(),
    );
    assert_eq!(result.status, "ready", "{}", result.detail);
    assert_eq!(result.server.as_deref(), Some(enabled.id.as_str()));
    let explicit = runtime.execute(
        fixture.workspace(),
        &[],
        Request::Definition {
            path: fixture.sample(),
            line: 1,
            column: 1,
            server: Some(disabled_id),
        },
        &Cancellation::default(),
    );
    assert_eq!(explicit.status, "off");
    store.set_enabled(&enabled.id, false).unwrap();
    let all_off = runtime.execute(
        fixture.workspace(),
        &[],
        request(),
        &Cancellation::default(),
    );
    assert_eq!(all_off.status, "off");
    runtime.shutdown();
}

#[test]
fn oversized_watched_file_reports_sync_gap_without_crashing_server() {
    let fixture = fixture::Fixture::new();
    let home = fixture.workspace().join("home");
    Store::new(home.clone()).save(fixture.profile()).unwrap();
    let runtime = Runtime::with_home(home);
    let request = || Request::Definition {
        path: fixture.sample(),
        line: 1,
        column: 1,
        server: None,
    };
    let first = runtime.execute(
        fixture.workspace(),
        &[],
        request(),
        &Cancellation::default(),
    );
    assert_eq!(first.status, "ready", "{}", first.detail);
    let pid = runtime.statuses()[0].pid;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !std::fs::read_to_string(fixture.marker())
        .unwrap()
        .contains("registration_reply")
    {
        assert!(
            std::time::Instant::now() < deadline,
            "watch registration not observed"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let oversized = fixture.workspace().join("huge.txt");
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(32 * 1024 * 1024 + 1)
        .unwrap();
    let result = runtime.execute(
        fixture.workspace(),
        &[],
        request(),
        &Cancellation::default(),
    );
    assert_eq!(result.status, "ready", "{}", result.detail);
    assert_eq!(result.freshness, "sync_gap");
    assert_eq!(runtime.statuses()[0].pid, pid);
    std::fs::remove_file(oversized).unwrap();
    let recovered = runtime.execute(
        fixture.workspace(),
        &[],
        request(),
        &Cancellation::default(),
    );
    assert_eq!(recovered.status, "ready");
    assert_eq!(runtime.statuses()[0].pid, pid);
    runtime.shutdown();
}

#[test]
fn cancelling_a_queued_call_does_not_dispatch_or_stop_shared_server() {
    let fixture = fixture::Fixture::new();
    let home = fixture.workspace().join("home");
    Store::new(home.clone()).save(fixture.profile()).unwrap();
    let runtime = Runtime::with_home(home);
    let warm = runtime.execute(
        fixture.workspace(),
        &[],
        Request::Definition {
            path: fixture.sample(),
            line: 1,
            column: 1,
            server: None,
        },
        &Cancellation::default(),
    );
    assert_eq!(warm.status, "ready");
    let pid = runtime.statuses()[0].pid;
    let first_runtime = runtime.clone();
    let first_path = fixture.sample();
    let first_root = fixture.workspace().to_path_buf();
    // The fixture holds this request until the release below.
    let first = std::thread::spawn(move || {
        first_runtime.execute(
            &first_root,
            &[],
            Request::Definition {
                path: first_path,
                line: 1,
                column: 10,
                server: None,
            },
            &Cancellation::default(),
        )
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::fs::read_to_string(fixture.marker())
        .unwrap()
        .matches("textDocument/definition")
        .count()
        < 2
    {
        assert!(
            std::time::Instant::now() < deadline,
            "Held call was not dispatched"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let queued_runtime = runtime.clone();
    let queued_path = fixture.sample();
    let queued_root = fixture.workspace().to_path_buf();
    let cancel = Cancellation::default();
    let queued_cancel = cancel.clone();
    let queued = std::thread::spawn(move || {
        queued_runtime.execute(
            &queued_root,
            &[],
            Request::Definition {
                path: queued_path,
                line: 1,
                column: 1,
                server: None,
            },
            &queued_cancel,
        )
    });
    std::thread::sleep(std::time::Duration::from_millis(30));
    cancel.cancel();
    let outcome = queued.join().unwrap();
    assert_eq!(outcome.status, "cancelled");
    // The held call cannot finish before its release, so it is still
    // running unless the cancellation waited for it to time out.
    assert!(
        !first.is_finished(),
        "cancellation waited for the held call"
    );
    fixture.release();
    assert_eq!(first.join().unwrap().status, "ready");
    assert_eq!(
        std::fs::read_to_string(fixture.marker())
            .unwrap()
            .matches("textDocument/definition")
            .count(),
        2
    );
    assert_eq!(runtime.statuses()[0].pid, pid);
    runtime.shutdown();
}
