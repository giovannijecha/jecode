use super::tests::{frame, read_client_frame};
use super::transport::{Cdp, base64};
use crate::cancel::Cancellation;
use crate::json::Value;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

#[test]
fn partial_frame_timeout_reconnects_tab_before_next_action() {
    use super::Browser;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let endpoint = format!("ws://127.0.0.1:{port}/devtools/page/fixture");
    let server = thread::spawn(move || {
        let handshake = |socket: &mut std::net::TcpStream| {
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap();
            let key = request
                .lines()
                .find_map(|line| line.strip_prefix("Sec-WebSocket-Key: "))
                .unwrap()
                .trim();
            let accept = base64(&super::transport::sha1(
                format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
            ));
            write!(
                socket,
                "HTTP/1.1 101 Switching Protocols\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
            )
            .unwrap();
        };
        let (mut first, _) = listener.accept().unwrap();
        handshake(&mut first);
        let _ = read_client_frame(&mut first);
        first.write_all(&[0x81, 32, b'{', b'"', b'i']).unwrap();
        let (mut http, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            http.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        let body = format!(
            "[{{\"id\":\"fixture\",\"type\":\"page\",\"webSocketDebuggerUrl\":\"ws://127.0.0.1:{port}/devtools/page/fixture\"}}]"
        );
        write!(
            http,
            "HTTP/1.1 200 OK\r\nContent-Length:{}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let (mut second, _) = listener.accept().unwrap();
        handshake(&mut second);
        second
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        for (index, method) in [
            "Page.enable",
            "Runtime.enable",
            "Network.enable",
            "Emulation.setDeviceMetricsOverride",
        ]
        .iter()
        .enumerate()
        {
            let id = index + 1;
            let (_, request) = read_client_frame(&mut second);
            let request = crate::json::parse(&String::from_utf8(request).unwrap()).unwrap();
            assert_eq!(request.get("id"), Some(&Value::number(id)));
            assert_eq!(request.get("method").and_then(Value::as_str), Some(*method));
            if *method == "Emulation.setDeviceMetricsOverride" {
                let params = request.get("params").unwrap();
                assert_eq!(params.get("width"), Some(&Value::number(1100)));
                assert_eq!(params.get("height"), Some(&Value::number(760)));
            }
            second
                .write_all(&frame(
                    1,
                    true,
                    format!("{{\"id\":{id},\"result\":{{}}}}").as_bytes(),
                ))
                .unwrap();
        }
    });
    let mut browser = Browser {
        supervisor: None,
        lifetime: Cancellation::default(),
        profile: std::path::PathBuf::new(),
        lease: None,
        port,
        tabs: std::collections::BTreeMap::new(),
        viewports: std::collections::BTreeMap::new(),
        id: "fixture".into(),
    };
    browser.viewports.insert("fixture".into(), (1100, 760));
    browser
        .tabs
        .insert("fixture".into(), Cdp::connect(&endpoint).unwrap());
    let cancellation = Cancellation::default();
    assert!(
        browser
            .tabs
            .get_mut("fixture")
            .unwrap()
            .call(
                "Page.navigate",
                Value::object([]),
                &cancellation,
                Duration::from_millis(100)
            )
            .unwrap_err()
            .contains("timed out")
    );
    assert!(!browser.tabs.get("fixture").unwrap().usable());
    let connection = super::actions::tab(&mut browser, "fixture", &cancellation).unwrap();
    assert!(connection.usable());
    let page = connection
        .events
        .page(super::logs::Filter::Default, 10, None)
        .unwrap();
    assert_eq!(page.entries.len(), 1, "the tab's log continues with a gap");
    assert_eq!(
        page.entries[0].get("kind").and_then(Value::as_str),
        Some("gap")
    );
    server.join().unwrap();
}
