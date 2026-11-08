use super::tests::{WAIT, panel, rows, settle, type_text};
use super::*;
use crate::{
    agent::Agent,
    backend::openrouter::OpenRouter,
    config::{Settings, Store as SettingsStore},
    session::SessionConfig,
    test_support::{
        Directory, HttpFixture,
        server::{Server, accept},
    },
    tools::Tools,
};
use std::{
    io::{Read, Write},
    net::TcpStream,
    time::{Duration, Instant},
};

fn app(directory: &Directory, fixture: &HttpFixture) -> App {
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let store = SettingsStore::new(directory.path().join("config"));
    store.save(&settings).unwrap();
    App::new(
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        ),
        SessionConfig { store, settings },
        None,
    )
}

fn request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
        let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&bytes[..end]);
        let length = head
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|n| n.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        if bytes.len() >= end + 4 + length {
            return String::from_utf8(bytes).unwrap();
        }
    }
}

fn reply(stream: &mut TcpStream, status: u16, body: &str) {
    write!(stream, "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{body}", body.len()).unwrap();
}

#[test]
fn saved_sign_in_worker_preserves_a_challenge_arriving_after_authorization_started() {
    let server = Server::start(|listener, stop| {
        let mut requests = Vec::new();
        for body in [
            r#"{"jsonrpc":"2.0","id":1,"result":{"supportedVersions":["2026-07-28"]}}"#,
            r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}"#,
        ] {
            let mut stream = accept(&listener, &stop).unwrap();
            requests.push(request(&mut stream));
            reply(&mut stream, 200, body);
        }
        requests
    });
    let directory = Directory::new();
    let store = crate::mcp::Store::new(directory.path().to_path_buf());
    let endpoint = &server.endpoint;
    let record = crate::json::parse(&format!(
        r#"{{"id":"mcp-fixture","name":"Scoped","endpoint":"{endpoint}","method":"browser","oauth":{{"access_token":"isolated-old","endpoint":"{endpoint}","client_id":"fixture-client","issuer":"{endpoint}","token_endpoint":"{endpoint}/token","resource":"{endpoint}","github":false}}}}"#
    ))
    .unwrap();
    let cached = store
        .save(&crate::mcp::config::read(&record).unwrap())
        .unwrap();
    store.require_scope(&cached, "deploy").unwrap();
    let (mut signed, guard) = store.prepare_sign_in(&cached, Some(&cached)).unwrap();
    assert_eq!(signed.pending_scope.as_deref(), Some("deploy"));
    let challenged = store.list().unwrap().remove(0);
    store.require_scope(&challenged, "admin").unwrap();
    signed.oauth.as_mut().unwrap().access_token = "isolated-new".into();
    signed.pending_scope = None;
    let result = Job::save(directory.path().to_path_buf(), signed, false, guard)
        .finish()
        .unwrap();
    let ResultValue::Saved(saved) = result else {
        panic!("saved sign-in worker returned an unexpected result");
    };
    let (saved, _) = saved.unwrap_or_else(|error| panic!("saved sign-in worker failed: {error:?}"));
    assert_eq!(saved.pending_scope.as_deref(), Some("admin"));
    assert_eq!(
        store.list().unwrap()[0]
            .oauth
            .as_ref()
            .unwrap()
            .access_token,
        "isolated-new"
    );
    assert_eq!(server.finish().len(), 2);
}

#[test]
fn token_connection_saves_and_inspects_real_loopback_tools_without_losing_edits() {
    let server = Server::start(|listener, stop| {
        let mut requests = Vec::new();
        while let Ok(mut stream) = accept(&listener, &stop) {
            let text = request(&mut stream);
            if !text
                .to_ascii_lowercase()
                .contains("authorization: bearer isolated-token")
            {
                reply(&mut stream, 401, r#"{"error":"unauthorized"}"#);
            } else if text.contains("\"method\":\"server/discover\"") {
                reply(
                    &mut stream,
                    200,
                    r#"{"jsonrpc":"2.0","id":1,"result":{"supportedVersions":["2026-07-28"],"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"fixture","version":"1"}}}}"#,
                );
            } else if text.contains("\"method\":\"tools/list\"") {
                reply(
                    &mut stream,
                    200,
                    r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"search","description":"Search the fixture repository","inputSchema":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}}]}}"#,
                );
            } else {
                panic!(
                    "Unexpected MCP request: {}",
                    text.lines().next().unwrap_or("")
                );
            }
            requests.push(text);
        }
        requests
    });
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.open_mcp(true);
    settle(&mut app);
    app.choose_mcp(Panel::List, 0); // Add connection
    app.choose_mcp(Panel::Add, PRESETS.len()); // Server URL
    app.state
        .selector
        .as_mut()
        .unwrap()
        .editor
        .replace(format!("{}/mcp", server.endpoint));
    app.save_mcp_field();
    app.choose_mcp(Panel::Form, 0);
    app.state
        .selector
        .as_mut()
        .unwrap()
        .editor
        .replace("Fixture GitHub".into());
    app.save_mcp_field();
    app.choose_mcp(Panel::Form, 2);
    settle(&mut app);
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        Purpose::Mcp(Panel::Methods)
    ));
    app.choose_mcp(Panel::Methods, 0); // Explicit Token choice, even for a token-only server.
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        Purpose::Mcp(Panel::Field(Field::Token))
    ));
    app.state
        .selector
        .as_mut()
        .unwrap()
        .editor
        .replace("isolated-token".into());
    app.save_mcp_field();
    app.choose_mcp(Panel::Form, 4); // Validate and save.
    settle(&mut app);
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        Purpose::Mcp(Panel::Detail(_))
    ));
    assert_eq!(
        crate::mcp::Store::new(app.mcp_home()).list().unwrap()[0].token,
        "isolated-token"
    );
    app.close_mcp_panel();
    app.close_mcp_panel();
    app.open_mcp(false);
    settle(&mut app);
    app.choose_mcp(Panel::List, 0);
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        Purpose::Mcp(Panel::Tools)
    ));
    app.choose_mcp(Panel::Tools, 0);
    let detail = app.state.information.as_ref().unwrap();
    assert!(
        detail.sections[0]
            .details
            .iter()
            .any(|(key, value)| key == "Description" && value.contains("Search the fixture"))
    );
    assert!(
        detail.sections[0]
            .details
            .iter()
            .any(|(key, value)| key == "Inputs" && value.contains("query"))
    );
    app.state.information = None;
    app.open_mcp(true);
    settle(&mut app);
    app.choose_mcp(Panel::List, 0);
    app.choose_mcp(Panel::Detail(Health::Unchecked), 0);
    app.choose_mcp(Panel::Form, 0);
    app.state
        .selector
        .as_mut()
        .unwrap()
        .editor
        .replace("Unsaved rename".into());
    app.save_mcp_field();
    app.close_mcp_panel(); // Cancel edit.
    assert_eq!(
        crate::mcp::Store::new(app.mcp_home()).list().unwrap()[0].name,
        "Fixture GitHub"
    );
    assert!(app.mcp.health.values().any(Result::is_ok));
    app.choose_mcp(Panel::Detail(Health::Unchecked), 0);
    app.choose_mcp(Panel::Form, 1);
    app.state
        .selector
        .as_mut()
        .unwrap()
        .editor
        .replace("http://127.0.0.1:9/other".into());
    app.save_mcp_field();
    assert!(app.mcp.connection().unwrap().token.is_empty());
    assert!(app.mcp.health.values().any(Result::is_ok));
    app.close_mcp_panel();
    assert_eq!(
        crate::mcp::Store::new(app.mcp_home()).list().unwrap()[0].token,
        "isolated-token"
    );
    let requests = server.finish();
    assert!(
        requests
            .iter()
            .any(|request| request.starts_with("POST ") && !request.contains("isolated-token"))
    );
    assert!(
        requests
            .iter()
            .any(|request| request.contains("Authorization: Bearer isolated-token"))
    );
    assert!(fixture.finish().is_empty());
}

#[test]
fn healthy_inventory_result_arrives_while_another_server_is_slow() {
    let (release, waiting) = std::sync::mpsc::channel();
    let slow = Server::start(move |listener, stop| {
        if let Ok(mut stream) = accept(&listener, &stop) {
            let _ = request(&mut stream);
            let _ = waiting.recv_timeout(WAIT);
        }
    });
    let fast = Server::start(|listener, stop| {
        let mut served = 0;
        while served < 2 {
            let Ok(mut stream) = accept(&listener, &stop) else {
                break;
            };
            let text = request(&mut stream);
            let body = if text.contains("\"method\":\"server/discover\"") {
                r#"{"jsonrpc":"2.0","id":1,"result":{"supportedVersions":["2026-07-28"]}}"#
            } else {
                r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"fast","inputSchema":{"type":"object"}}]}}"#
            };
            reply(&mut stream, 200, body);
            served += 1;
        }
        served
    });
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let store = crate::mcp::Store::new(app.mcp_home());
    let slow_saved = store
        .save(&Connection::draft(
            "Slow",
            &format!("{}/mcp", slow.endpoint),
        ))
        .unwrap();
    let fast_saved = store
        .save(&Connection::draft(
            "Fast",
            &format!("{}/mcp", fast.endpoint),
        ))
        .unwrap();
    app.open_mcp(false);
    let deadline = Instant::now() + WAIT;
    while !app.mcp.health.contains_key(&fast_saved.id) && Instant::now() < deadline {
        app.poll_mcp();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        app.mcp
            .health
            .get(&fast_saved.id)
            .is_some_and(Result::is_ok)
    );
    assert!(!app.mcp.health.contains_key(&slow_saved.id));
    assert!(app.mcp.checks.pending(&slow_saved.id));
    let _ = release.send(());
    app.close_mcp_panel();
    assert_eq!(fast.finish(), 2);
    slow.finish();
    assert!(fixture.finish().is_empty());
}

#[test]
fn background_checks_survive_navigation_between_panels() {
    let (release, waiting) = std::sync::mpsc::channel();
    let slow = Server::start(move |listener, stop| {
        if let Ok(mut stream) = accept(&listener, &stop) {
            let _ = request(&mut stream);
            let _ = waiting.recv_timeout(WAIT);
        }
    });
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let saved = crate::mcp::Store::new(app.mcp_home())
        .save(&Connection::draft(
            "Slow",
            &format!("{}/mcp", slow.endpoint),
        ))
        .unwrap();
    app.open_mcp(true);
    while app.mcp.job.is_some() {
        app.poll_mcp();
    }
    assert_eq!(rows(&app)[0].1, "checking…");
    app.choose_mcp(Panel::List, 0);
    assert_eq!(panel(&app), Panel::Detail(Health::Checking));
    app.choose_mcp(Panel::Detail(Health::Checking), 0);
    app.close_mcp_panel();
    app.close_mcp_panel();
    app.choose_mcp(Panel::List, 1);
    app.close_mcp_panel();
    assert!(app.mcp.checks.pending(&saved.id));
    let _ = release.send(());
    let deadline = Instant::now() + WAIT;
    while !app.mcp.health.contains_key(&saved.id) && Instant::now() < deadline {
        app.poll_mcp();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(rows(&app)[0].1, "unreachable");
    app.close_mcp_panel();
    slow.finish();
    assert!(fixture.finish().is_empty());
}

#[test]
fn saving_an_edit_keeps_oauth_tokens_rotated_since_it_was_loaded() {
    let server = Server::start(|listener, stop| {
        while let Ok(mut stream) = accept(&listener, &stop) {
            let text = request(&mut stream);
            if !text.contains("Bearer isolated-new-access") {
                reply(&mut stream, 401, r#"{"error":"unauthorized"}"#);
            } else if text.contains("\"method\":\"server/discover\"") {
                reply(
                    &mut stream,
                    200,
                    r#"{"jsonrpc":"2.0","id":1,"result":{"supportedVersions":["2026-07-28"]}}"#,
                );
            } else {
                reply(
                    &mut stream,
                    200,
                    r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"fixture","inputSchema":{"type":"object"}}]}}"#,
                );
            }
        }
    });
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let store = crate::mcp::Store::new(app.mcp_home());
    let endpoint = format!("{}/mcp", server.endpoint);
    std::fs::write(
        app.mcp_home().join("mcp.json"),
        format!(
            r#"{{"connections":[{{"id":"mcp-fixture","name":"OAuth fixture","endpoint":"{endpoint}","enabled":true,"method":"browser","token":"","timeout_secs":5,"headers":{{}},"oauth":{{"access_token":"isolated-old-access","refresh_token":"isolated-old-refresh","expires_at":4000000000,"token_endpoint":"http://127.0.0.1:9/token","endpoint":"{endpoint}","client_id":"fixture-client","issuer":"http://127.0.0.1:9","scope":null,"resource":"{endpoint}","github":false}}}}]}}"#
        ),
    )
    .unwrap();
    app.open_mcp(true);
    settle(&mut app);
    assert_eq!(rows(&app)[0].1, "sign in again");
    // A refresh elsewhere rotates both tokens while the edit is open.
    app.choose_mcp(Panel::List, 0);
    app.choose_mcp(Panel::Detail(Health::Expired), 0);
    let mut rotated = store.list().unwrap().remove(0);
    let tokens = rotated.oauth.as_mut().unwrap();
    tokens.access_token = "isolated-new-access".into();
    tokens.refresh_token = Some("isolated-new-refresh".into());
    store.save(&rotated).unwrap();
    app.choose_mcp(Panel::Form, 0);
    type_text(&mut app, "OAuth renamed");
    app.choose_mcp(Panel::Form, 4);
    settle(&mut app);
    assert_eq!(panel(&app), Panel::Detail(Health::Available(1)));
    let stored = store.list().unwrap().remove(0);
    assert_eq!(stored.name, "OAuth renamed");
    assert!(stored.oauth == rotated.oauth);
    assert!(app.selected_mcp().unwrap().oauth == rotated.oauth);
    server.finish();
    assert!(fixture.finish().is_empty());
}
