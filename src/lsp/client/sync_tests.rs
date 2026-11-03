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
