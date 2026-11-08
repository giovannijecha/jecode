use super::*;
use crate::cancel::Cancellation;

#[test]
fn single_request_replies_open_a_new_connection_for_the_next_rpc() {
    let server = Server::start(|listener, stop| {
        // Hold completed sockets open to expose their advertised lifetime,
        // independently of when the peer observes a FIN after the reply.
        let mut completed = Vec::new();
        let mut requests = Vec::new();
        for body in [
            r#"{"jsonrpc":"2.0","id":1,"result":{"supportedVersions":["2026-07-28"]}}"#,
            r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}"#,
        ] {
            let Ok(mut stream) = accept(&listener, &stop) else {
                break;
            };
            requests.push(request(&mut stream));
            reply(&mut stream, 200, body);
            completed.push(stream);
        }
        requests
    });
    let mut connection = crate::mcp::Connection::draft("Fixture", &server.endpoint);
    connection.method = crate::mcp::AuthMethod::None;
    connection.timeout_secs = 10;
    let result = crate::mcp::Client::new(connection).catalog(&Cancellation::default());
    let requests = server.finish();
    let catalog = result.unwrap_or_else(|error| {
        panic!(
            "fixture RPC failed after {} connections: {error:?}",
            requests.len()
        )
    });
    assert!(catalog.tools.is_empty());
    assert_eq!(requests.len(), 2);
    assert!(requests[0].contains("server/discover"));
    assert!(requests[1].contains("tools/list"));
}
