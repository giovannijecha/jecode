//! Client synchronization against the owned fixture server.
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::lsp::client::Client;
use crate::lsp::documents::path_to_uri;
use crate::lsp::fixture::Fixture;
use crate::test_support::Directory;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

fn start(fixture: &Fixture) -> (Client, PathBuf) {
    let root = std::fs::canonicalize(fixture.workspace()).unwrap();
    let client = Client::start(
        &fixture.profile(),
        &root,
        &Cancellation::default(),
        &Cancellation::default(),
        1,
    )
    .unwrap();
    (client, root)
}

fn companion(fixture: &Fixture, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}-{suffix}", fixture.marker().to_string_lossy()))
}

fn definition(client: &mut Client, path: &Path) -> Value {
    let uri = path_to_uri(&std::fs::canonicalize(path).unwrap());
    client
        .query(
            "textDocument/definition",
            Value::object([
                ("textDocument", Value::object([("uri", Value::string(uri))])),
                (
                    "position",
                    Value::object([("line", Value::number(0)), ("character", Value::number(0))]),
                ),
            ]),
            Duration::from_secs(5),
            &Cancellation::default(),
            &Cancellation::default(),
        )
        .unwrap()
}

/// Process the fixture's own watch registration so that it cannot reset the
/// watch baseline in the middle of a test.
fn settle(client: &mut Client) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !client.registrations.contains_key("watch") {
        assert!(Instant::now() < deadline, "fixture registration missing");
        client.drain().unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn register(client: &mut Client, id: &str, glob: Value) {
    let watcher = Value::object([("globPattern", glob), ("kind", Value::number(7))]);
    let registration = Value::object([
        ("id", Value::string(id)),
        ("method", Value::string("workspace/didChangeWatchedFiles")),
        (
            "registerOptions",
            Value::object([("watchers", Value::Array(vec![watcher]))]),
        ),
    ]);
    client
        .handle(Value::object([
            ("jsonrpc", Value::string("2.0")),
            ("id", Value::number(7000)),
            ("method", Value::string("client/registerCapability")),
            (
                "params",
                Value::object([("registrations", Value::Array(vec![registration]))]),
            ),
        ]))
        .unwrap();
    assert!(client.registrations.contains_key(id));
}

/// An absolute path as a server spells it in a glob, without a verbatim prefix.
fn spelled(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    text.strip_prefix("//?/").map(str::to_owned).unwrap_or(text)
}

fn change(kind: u8, path: &Path) -> String {
    format!(r#"{{"type":{kind},"uri":"{}"}}"#, path_to_uri(path))
}

/// Every didChangeWatchedFiles the fixture received; the query round trip
/// guarantees that notifications sent before it were logged.
fn watched_log(client: &mut Client, fixture: &Fixture) -> String {
    definition(client, &fixture.sample());
    std::fs::read_to_string(companion(fixture, "watched")).unwrap_or_default()
}

fn set_modified(path: &Path, time: SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

#[test]
fn generated_folders_stay_outside_the_watch_scan_budget() {
    let fixture = Fixture::new();
    let (mut client, root) = start(&fixture);
    settle(&mut client);
    let deps = root.join("target").join("debug").join("deps");
    std::fs::create_dir_all(&deps).unwrap();
    for index in 0..10_050 {
        std::fs::write(deps.join(format!("unit{index}.rs")), "").unwrap();
    }
    for folder in ["node_modules", ".tmp", ".jecode-1.tmp"] {
        std::fs::create_dir(root.join(folder)).unwrap();
        std::fs::write(root.join(folder).join("skipped.rs"), "").unwrap();
    }
    std::fs::create_dir(root.join("src")).unwrap();
    let source = root.join("src").join("lib.rs");
    std::fs::write(&source, "pub fn lib() {}\n").unwrap();
    let kept = root.join("target").join("keep");
    std::fs::create_dir(&kept).unwrap();
    let output = kept.join("out.json");
    std::fs::write(&output, "{}").unwrap();
    register(
        &mut client,
        "kept",
        Value::string(format!("{}/*.json", spelled(&kept))),
    );
    let relative = root.join("target").join("relative");
    std::fs::create_dir(&relative).unwrap();
    register(
        &mut client,
        "relative",
        Value::object([
            ("baseUri", Value::string(path_to_uri(&root))),
            ("pattern", Value::string("target/relative/*.json")),
        ]),
    );
    let recursive = root.join("target").join("recursive");
    std::fs::create_dir(&recursive).unwrap();
    register(
        &mut client,
        "recursive",
        Value::string("**/target/recursive/*.json"),
    );
    client.poll_watches().unwrap();
    assert!(!client.sync_gap);
    assert!(client.watched.contains_key(&source) && client.watched.contains_key(&output));
    assert!(
        client.watched.keys().all(|path| {
            !path.starts_with(&deps) && !path.to_string_lossy().contains("skipped")
        })
    );
    std::fs::write(&source, "pub fn changed_lib() {}\n").unwrap();
    std::fs::write(&output, r#"{"changed":true}"#).unwrap();
    let relative_output = relative.join("new.json");
    std::fs::write(&relative_output, "{}").unwrap();
    let recursive_output = recursive.join("new.json");
    std::fs::write(&recursive_output, "{}").unwrap();
    client.poll_watches().unwrap();
    assert!(!client.sync_gap);
    let log = watched_log(&mut client, &fixture);
    assert!(log.contains(&change(2, &source)), "{log}");
    assert!(log.contains(&change(2, &output)), "{log}");
    assert!(log.contains(&change(1, &relative_output)), "{log}");
    assert!(log.contains(&change(1, &recursive_output)), "{log}");
    std::fs::write(&relative_output, r#"{"changed":true}"#).unwrap();
    std::fs::write(&recursive_output, r#"{"changed":true}"#).unwrap();
    client.poll_watches().unwrap();
    assert!(!client.sync_gap);
    let log = watched_log(&mut client, &fixture);
    assert!(log.contains(&change(2, &relative_output)), "{log}");
    assert!(log.contains(&change(2, &recursive_output)), "{log}");
    client.shutdown();
}

#[test]
fn reregistration_still_reports_changes_to_tracked_paths() {
    let fixture = Fixture::new();
    let (mut client, root) = start(&fixture);
    settle(&mut client);
    let notes = root.join("notes");
    std::fs::create_dir(&notes).unwrap();
    let [edited, removed, created, matched] =
        ["edited.txt", "removed.txt", "created.txt", "matched.md"].map(|name| notes.join(name));
    for path in [&edited, &removed, &matched] {
        std::fs::write(path, "one").unwrap();
    }
    client.poll_watches().unwrap();
    assert!(client.watched.contains_key(&edited));
    std::fs::write(&edited, "two, longer").unwrap();
    std::fs::remove_file(&removed).unwrap();
    std::fs::write(&created, "new").unwrap();
    // A workspace reload re-registers the same watcher with a wider pattern.
    register(&mut client, "watch", Value::string("**/*.{txt,md}"));
    client.poll_watches().unwrap();
    let log = watched_log(&mut client, &fixture);
    assert!(log.contains(&change(2, &edited)), "{log}");
    assert!(log.contains(&change(3, &removed)), "{log}");
    for baseline in [&created, &matched] {
        assert!(!log.contains(&path_to_uri(baseline)), "{log}");
    }
    std::fs::write(&created, "newer text").unwrap();
    client.poll_watches().unwrap();
    let log = watched_log(&mut client, &fixture);
    assert!(log.contains(&change(2, &created)), "{log}");
    client.shutdown();
}

#[test]
fn missing_watch_bases_do_not_hold_back_other_watchers() {
    let fixture = Fixture::new();
    let (mut client, root) = start(&fixture);
    settle(&mut client);
    let outside = Directory::new();
    let tracked = root.join("tracked.txt");
    std::fs::write(&tracked, "one").unwrap();
    register(
        &mut client,
        "outside",
        Value::object([
            (
                "baseUri",
                Value::string(path_to_uri(&outside.path().join("missing"))),
            ),
            ("pattern", Value::string("**/*.cfg")),
        ]),
    );
    register(
        &mut client,
        "generated",
        Value::string(format!("{}/gen/**/*.json", spelled(&root))),
    );
    let base = &client.registrations["generated"][0].base;
    assert!(base.starts_with(&root), "{}", base.display());
    client.poll_watches().unwrap();
    assert!(!client.sync_gap);
    std::fs::write(&tracked, "two, longer").unwrap();
    std::fs::create_dir(root.join("gen")).unwrap();
    let generated = root.join("gen").join("out.json");
    std::fs::write(&generated, "{}").unwrap();
    client.poll_watches().unwrap();
    assert!(!client.sync_gap);
    let log = watched_log(&mut client, &fixture);
    assert!(log.contains(&change(2, &tracked)), "{log}");
    assert!(log.contains(&change(1, &generated)), "{log}");
    assert_eq!(log.matches(&path_to_uri(&generated)).count(), 1, "{log}");
    client.shutdown();
}

#[test]
fn settled_files_revalidate_after_metadata_change_or_interval() {
    let fixture = Fixture::new();
    let (mut client, root) = start(&fixture);
    let old = SystemTime::now() - Duration::from_secs(60);
    let [settled, recent] = ["settled.rs", "recent.rs"].map(|name| root.join(name));
    std::fs::write(&settled, "one").unwrap();
    set_modified(&settled, old);
    std::fs::write(&recent, "one").unwrap();
    client.poll_watches().unwrap();
    let hash = |client: &Client, path: &Path| client.watched[path].hash;
    let (first_settled, first_recent) = (hash(&client, &settled), hash(&client, &recent));
    // Same size and timestamp: a settled hash is trusted briefly.
    std::fs::write(&settled, "two").unwrap();
    set_modified(&settled, old);
    // A hash taken right after a write is not trusted yet.
    let recent_time = std::fs::metadata(&recent).unwrap().modified().unwrap();
    std::fs::write(&recent, "two").unwrap();
    set_modified(&recent, recent_time);
    client.poll_watches().unwrap();
    assert_eq!(hash(&client, &settled), first_settled);
    assert_ne!(hash(&client, &recent), first_recent);
    client.watched.get_mut(&settled).unwrap().checked =
        Instant::now() - super::RECHECK - Duration::from_secs(1);
    client.poll_watches().unwrap();
    assert_ne!(hash(&client, &settled), first_settled);
    let log = watched_log(&mut client, &fixture);
    assert!(log.contains(&change(2, &settled)), "{log}");
    let refreshed = hash(&client, &settled);
    std::fs::write(&settled, "six").unwrap();
    set_modified(&settled, old + Duration::from_secs(1));
    client.poll_watches().unwrap();
    assert_ne!(hash(&client, &settled), refreshed);
    client.shutdown();
}

#[test]
fn full_text_changes_reconstruct_the_disk_text_for_every_line_break() {
    let fixture = Fixture::new();
    let (mut client, root) = start(&fixture);
    let document = root.join("mirror.rs");
    let mirror = companion(&fixture, "mirror");
    for content in [
        "fn a() {}\r\nfn b() {}\r\n",
        "x\ry\rz",
        "\u{feff}fn bom() {}\n",
        "fn 😀() {}\r\n// 𝄞\rend",
        "a\r\n\rb\n\r",
    ] {
        std::fs::write(&document, content).unwrap();
        client.reconcile(&document, "rust").unwrap();
        definition(&mut client, &document);
        assert_eq!(
            std::fs::read(&mirror).unwrap(),
            std::fs::read(&document).unwrap(),
            "server copy diverged after writing {content:?}"
        );
    }
    client.shutdown();
}

#[test]
fn unreadable_open_document_is_closed_without_stopping_the_server() {
    let fixture = Fixture::new();
    let (mut client, root) = start(&fixture);
    let pid = client.pid();
    let broken = root.join("sample.rs");
    let healthy = root.join("other.rs");
    std::fs::write(&healthy, "pub fn other() {}\n").unwrap();
    let uri = client.reconcile(&broken, "rust").unwrap().uri;
    client.reconcile(&healthy, "rust").unwrap();
    client.push_diagnostics.insert(
        uri.clone(),
        crate::lsp::client::DiagnosticReport {
            values: Vec::new(),
            version: Some(1),
            result_id: None,
            generation: 1,
            stale: false,
        },
    );
    std::fs::write(&broken, b"pub fn \xff() {}\n").unwrap();
    client.poll_open_documents().unwrap();
    assert_eq!(client.pid(), pid);
    assert!(!client.docs.contains_key(&uri));
    assert!(!client.push_diagnostics.contains_key(&uri));
    assert!(definition(&mut client, &healthy).get("result").is_some());
    let log = std::fs::read_to_string(fixture.marker()).unwrap();
    assert!(log.contains("textDocument/didClose"), "{log}");
    let error = client.reconcile(&broken, "rust").err().unwrap();
    assert!(
        error.starts_with("Cannot read UTF-8 LSP document"),
        "{error}"
    );
    assert_eq!(client.pid(), pid);
    client.shutdown();
}
