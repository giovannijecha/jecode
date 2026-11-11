//! A catalog of any page count and size loads; a repeated cursor still fails.
use super::*;

const DISCOVER: &str = r#"{"supportedVersions":["2026-07-28"],"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"big","version":"1"}}}"#;

/// Answers discovery, then each page in turn.
fn catalog_server(pages: Vec<String>) -> Server<usize> {
    Server::start(move |listener, stop| {
        let mut stream = accept(&listener, &stop).unwrap();
        read_request(&mut stream);
        reply(&mut stream, 200, "", &fixture_response(1, DISCOVER));
        let count = pages.len();
        for (index, page) in pages.into_iter().enumerate() {
            let mut stream = accept(&listener, &stop).unwrap();
            read_request(&mut stream);
            reply(&mut stream, 200, "", &fixture_response(index + 2, &page));
        }
        count
    })
}

fn page(index: usize, size: usize, next: Option<String>) -> String {
    let tools = (0..size)
        .map(|tool| format!(r#"{{"name":"t{index}_{tool}","inputSchema":{{"type":"object"}}}}"#))
        .collect::<Vec<_>>()
        .join(",");
    match next {
        Some(cursor) => format!(r#"{{"tools":[{tools}],"nextCursor":"{cursor}"}}"#),
        None => format!(r#"{{"tools":[{tools}]}}"#),
    }
}

#[test]
fn a_catalog_past_the_former_page_and_tool_counts_loads_whole() {
    // 120 pages of 100 tools: past the former 100 pages and 10,000 tools.
    let pages: Vec<String> = (0..120)
        .map(|index| page(index, 100, (index < 119).then(|| format!("c{index}"))))
        .collect();
    let server = catalog_server(pages);
    let mut client = Client::new(Connection::draft("big", &server.endpoint));
    let catalog = client.catalog(&Cancellation::default()).unwrap();
    assert_eq!(catalog.tools.len(), 12_000);
    assert_eq!(catalog.tools[11_999].name, "t119_99");
    assert_eq!(server.finish(), 120);
}

#[test]
fn a_repeated_cursor_and_cancellation_still_end_the_listing() {
    let pages = vec![
        page(0, 1, Some("again".into())),
        page(1, 1, Some("again".into())),
    ];
    let server = catalog_server(pages);
    let mut client = Client::new(Connection::draft("loop", &server.endpoint));
    let error = client.catalog(&Cancellation::default()).unwrap_err();
    assert!(
        error.message.contains("Repeated MCP pagination cursor"),
        "{}",
        error.message
    );
    server.finish();

    let cancel = Cancellation::default();
    cancel.cancel();
    let mut client = Client::new(Connection::draft("stopped", "http://127.0.0.1:9/mcp"));
    let error = client.catalog(&cancel).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Cancelled);
}
