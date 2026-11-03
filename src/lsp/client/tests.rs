use super::*;
use crate::lsp::{Request, Runtime, Store, fixture::Fixture};
use std::ffi::OsStr;

#[test]
fn drain_yields_before_consuming_an_entire_notification_burst() {
    let fixture = Fixture::new();
    let path = fixture.workspace().join("burst.rs");
    std::fs::write(&path, "pub fn burst() {}\n").unwrap();
    let cancel = Cancellation::default();
    let stop = Cancellation::default();
    let mut client =
        Client::start(&fixture.profile(), fixture.workspace(), &cancel, &stop, 1).unwrap();
    let params = Value::object([(
        "textDocument",
        Value::object([(
            "uri",
            Value::string(crate::lsp::documents::path_to_uri(&path)),
        )]),
    )]);
    client
        .query(
            "textDocument/definition",
            params,
            Duration::from_secs(3),
            &cancel,
            &stop,
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while client.wire.queued_frames() < 1000 {
        assert!(Instant::now() < deadline, "Fixture burst was not queued");
        std::thread::sleep(Duration::from_millis(10));
    }
    client.drain().unwrap();
    assert!(client.wire.queued_frames() >= 744);
    client.shutdown();
}

#[test]
fn off_finishes_while_the_server_streams_notifications() {
    let fixture = Fixture::new();
    let runtime = started(&fixture, &["stream.rs"]);
    let outcome = definition(&runtime, &fixture, "stream.rs");
    assert_eq!(outcome.status, "ready", "{}", outcome.detail);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !std::fs::read_to_string(fixture.marker())
        .unwrap_or_default()
        .contains("streaming")
    {
        assert!(Instant::now() < deadline, "Fixture did not start streaming");
        std::thread::sleep(Duration::from_millis(10));
    }
    let server = runtime.statuses()[0].server.clone();
    let started = Instant::now();
    runtime.configuration_changed(&server);
    assert!(started.elapsed() < Duration::from_millis(200));
    drop(runtime);
    assert!(started.elapsed() < Duration::from_secs(5));
}

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

fn diagnostics(runtime: &Runtime, fixture: &Fixture, name: &str) -> crate::lsp::Outcome {
    runtime.execute(
        fixture.workspace(),
        &[],
        Request::Diagnostics {
            path: fixture.workspace().join(name),
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
fn diagnostic_eviction_gap_is_scoped_to_evicted_documents() {
    let fixture = Fixture::new();
    let runtime = started(&fixture, &["clear.rs", "flood.rs", "fresh.rs"]);
    assert_eq!(definition(&runtime, &fixture, "sample.rs").status, "ready");
    std::fs::write(fixture.sample(), "pub fn changed() {}\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let report = diagnostics(&runtime, &fixture, "sample.rs");
        if report.freshness == "current" && report.diagnostics.len() == 1 {
            break;
        }
        assert!(Instant::now() < deadline, "No push finding: {report:?}");
        std::thread::sleep(Duration::from_millis(25));
    }
    // Empty pushes for files that are not open must not evict the open file's findings.
    assert_eq!(definition(&runtime, &fixture, "clear.rs").status, "ready");
    let kept = diagnostics(&runtime, &fixture, "sample.rs");
    assert_eq!(kept.freshness, "current", "{kept:?}");
    assert_eq!(kept.diagnostics.len(), 1);
    assert_eq!(definition(&runtime, &fixture, "flood.rs").status, "ready");
    assert_eq!(
        diagnostics(&runtime, &fixture, "flood.rs").freshness,
        "sync_gap"
    );
    assert_eq!(
        diagnostics(&runtime, &fixture, "sample.rs").freshness,
        "sync_gap"
    );
    assert_eq!(definition(&runtime, &fixture, "fresh.rs").status, "ready");
    let fresh = diagnostics(&runtime, &fixture, "fresh.rs");
    assert_eq!(fresh.freshness, "current", "{}", fresh.detail);
    std::fs::write(fixture.workspace().join("flood.rs"), "pub fn edited() {}\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let report = diagnostics(&runtime, &fixture, "flood.rs");
        if report.freshness == "current" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Fresh push kept the gap: {report:?}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    runtime.shutdown();
}

#[test]
fn configuration_resolves_literal_then_dotted_sections_then_all_settings() {
    let settings = crate::json::parse(
        r#"{"python":{"analysis":{"typeCheckingMode":"strict"}},"rust-analyzer.cargo":1,"check":{"command":"clippy"}}"#,
    )
    .unwrap();
    let analysis = configuration_section(&settings, Some("python.analysis"));
    assert_eq!(
        analysis.get("typeCheckingMode").and_then(Value::as_str),
        Some("strict")
    );
    let literal = configuration_section(&settings, Some("rust-analyzer.cargo"));
    assert_eq!(literal.as_usize(), Some(1));
    for section in [Some("rust-analyzer"), Some("python.missing"), None] {
        assert_eq!(configuration_section(&settings, section), settings);
    }
}

#[test]
fn server_errors_keep_their_code_and_retry_hint() {
    let failure = |code: &str| {
        let message = crate::json::parse(&format!(
            r#"{{"id":1,"error":{{"code":{code},"message":"busy"}}}}"#
        ))
        .unwrap();
        response(message).unwrap_err()
    };
    assert_eq!(
        failure("-32801"),
        "LSP server error -32801: busy (content modified; retrying may succeed)"
    );
    assert!(failure("-32800").contains("retrying may succeed"));
    assert!(failure("-32802").contains("retrying may succeed"));
    assert_eq!(failure("-32803"), "LSP server error -32803: busy");
    assert_eq!(failure("\"x\""), "LSP server error: busy");
}

#[test]
fn capabilities_do_not_request_work_done_progress() {
    assert!(client_capabilities().get("window").is_none());
}

#[test]
fn server_command_withholds_provider_credentials_but_keeps_profile_variables() {
    let names = crate::backend::available()
        .iter()
        .flat_map(|backend| backend.credential_env.iter().copied())
        .collect::<Vec<_>>();
    let mut profile = Profile::draft_guided_rust();
    profile
        .environment
        .insert(names[0].into(), "profile value".into());
    let command = server_command(PathBuf::from("server"), &profile, Path::new("."));
    let value = |name: &str| {
        command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new(name))
            .map(|(_, value)| value)
    };
    assert_eq!(value(names[0]), Some(Some(OsStr::new("profile value"))));
    for name in names.iter().filter(|name| **name != names[0]) {
        assert_eq!(value(name), Some(None), "{name} reached the server");
    }
}
