//! Client synchronization against the owned fixture server.
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::lsp::client::Client;
use crate::lsp::documents::path_to_uri;
use crate::lsp::fixture::Fixture;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
