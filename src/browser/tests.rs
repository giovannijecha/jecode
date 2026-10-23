use super::transport::{Cdp, base64, http_get};
use crate::cancel::Cancellation;
use crate::json::Value;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

pub(super) fn frame(opcode: u8, final_fragment: bool, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![(if final_fragment { 0x80 } else { 0 }) | opcode];
    if payload.len() < 126 {
        bytes.push(payload.len() as u8);
    } else {
        bytes.push(126);
        bytes.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    }
    bytes.extend_from_slice(payload);
    bytes
}

#[test]
fn devtools_http_discovery_preserves_port_in_host() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        assert!(
            String::from_utf8(request)
                .unwrap()
                .contains(&format!("Host: 127.0.0.1:{port}\r\n"))
        );
        let body =
            format!("{{\"webSocketDebuggerUrl\":\"ws://127.0.0.1:{port}/devtools/browser/test\"}}");
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length:{}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        thread::sleep(Duration::from_millis(100));
    });
    let value = http_get(port, "/json/version").unwrap();
    assert!(
        value
            .get("webSocketDebuggerUrl")
            .and_then(Value::as_str)
            .unwrap()
            .contains(&format!(":{port}/"))
    );
    server.join().unwrap();
}

pub(super) fn read_client_frame(stream: &mut std::net::TcpStream) -> (u8, Vec<u8>) {
    let mut header = [0; 2];
    stream.read_exact(&mut header).unwrap();
    assert_ne!(header[1] & 0x80, 0, "client frames must be masked");
    let len = match header[1] & 0x7f {
        n @ 0..=125 => n as usize,
        126 => {
            let mut n = [0; 2];
            stream.read_exact(&mut n).unwrap();
            u16::from_be_bytes(n) as usize
        }
        _ => panic!("unexpected test frame"),
    };
    let mut mask = [0; 4];
    stream.read_exact(&mut mask).unwrap();
    let mut payload = vec![0; len];
    stream.read_exact(&mut payload).unwrap();
    for (i, b) in payload.iter_mut().enumerate() {
        *b ^= mask[i % 4];
    }
    (header[0] & 0xf, payload)
}

#[test]
fn websocket_masks_requests_handles_ping_fragments_events_and_response_ids() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
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
        write!(stream,"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").unwrap();
        let (opcode, request) = read_client_frame(&mut stream);
        assert_eq!(opcode, 1);
        assert!(String::from_utf8(request).unwrap().contains("\"id\":1"));
        stream.write_all(&frame(9, true, b"hi")).unwrap();
        let event = br#"{"method":"Page.frameNavigated","params":{"frame":{"id":"f"}}}"#;
        stream.write_all(&frame(1, false, &event[..20])).unwrap();
        stream.write_all(&frame(0, true, &event[20..])).unwrap();
        stream
            .write_all(&frame(1, true, br#"{"id":99,"result":{"wrong":true}}"#))
            .unwrap();
        stream
            .write_all(&frame(1, true, br#"{"id":1,"result":{"ok":true}}"#))
            .unwrap();
        let (opcode, pong) = read_client_frame(&mut stream);
        assert_eq!((opcode, pong), (10, b"hi".to_vec()));
        let (_, request) = read_client_frame(&mut stream);
        assert!(String::from_utf8(request).unwrap().contains("\"id\":2"));
        stream
            .write_all(&frame(
                1,
                true,
                br#"{"id":2,"error":{"message":"bad call"}}"#,
            ))
            .unwrap();
    });
    let mut cdp = Cdp::connect(&format!("ws://127.0.0.1:{port}/devtools/page/test")).unwrap();
    let result = cdp
        .call(
            "Page.enable",
            Value::object([]),
            &Cancellation::default(),
            Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(result.get("ok"), Some(&Value::Bool(true)));
    assert_eq!(cdp.events.len(), 1);
    let error = cdp
        .call(
            "Bad.call",
            Value::object([]),
            &Cancellation::default(),
            Duration::from_secs(5),
        )
        .unwrap_err();
    assert!(error.contains("bad call"));
    server.join().unwrap();
}

#[test]
fn cancel_stops_a_waiting_devtools_call() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
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
            stream,
            "HTTP/1.1 101 Switching Protocols\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
        )
        .unwrap();
        let _ = read_client_frame(&mut stream);
        thread::sleep(Duration::from_millis(400));
    });
    let mut cdp = Cdp::connect(&format!("ws://127.0.0.1:{port}/test")).unwrap();
    let cancellation = Cancellation::default();
    let second = cancellation.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        second.cancel();
    });
    assert!(
        cdp.call(
            "Wait",
            Value::object([]),
            &cancellation,
            Duration::from_secs(5)
        )
        .unwrap_err()
        .contains("cancelled")
    );
    assert!(!cdp.usable());
    server.join().unwrap();
}

#[test]
#[ignore = "requires an installed Chrome or Edge; creates an isolated browser profile"]
fn live_chromium_localhost_interaction_and_profile_persistence() {
    use super::Registry;
    use crate::scratch::Area;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    let server = thread::spawn(move || {
        while flag.load(Ordering::Relaxed) {
            if let Ok((mut socket, _)) = listener.accept() {
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut input = Vec::new();
                while !input.ends_with(b"\r\n\r\n") && input.len() < 8192 {
                    let mut byte = [0];
                    match socket.read(&mut byte) {
                        Ok(1) => input.push(byte[0]),
                        _ => break,
                    }
                }
                if !input.ends_with(b"\r\n\r\n") {
                    continue;
                }
                let body = r#"<!doctype html><title>Browser fixture</title><input id="entry"><button id="go" style="margin-top:1600px" onclick="localStorage.setItem('saved',document.querySelector('#entry').value);document.querySelector('#out').textContent=document.querySelector('#entry').value">Go</button><p id="out"></p>"#;
                let _ = write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }
    });
    let project = std::env::current_dir().unwrap();
    let fixture = project
        .join(".tmp")
        .join(format!("browser-fixture-{}", std::process::id()));
    let session = "123456-789";
    let area = Area::new(fixture.join(session), project, session).unwrap();
    let profiles = fixture.join("persistent");
    let registry = Registry::default();
    let cancel = Cancellation::default();
    let url = format!("http://127.0.0.1:{port}/");
    let open = registry
        .execute(
            &area,
            Some(&profiles),
            &Value::object([
                ("action", Value::string("open")),
                ("url", Value::string(&url)),
                ("profile", Value::string("fixture")),
            ]),
            &cancel,
        )
        .unwrap();
    let tab = open
        .get("tab_id")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let args = |action: &str, fields: Vec<(&str, Value)>| {
        let mut value = Value::object([
            ("action", Value::string(action)),
            ("tab_id", Value::string(&tab)),
        ]);
        if let Value::Object(ref mut entries) = value {
            for (key, value) in fields {
                entries.insert(key.into(), value);
            }
        }
        value
    };
    registry
        .execute(
            &area,
            Some(&profiles),
            &args("wait", vec![("selector", Value::string("#entry"))]),
            &cancel,
        )
        .unwrap();
    registry
        .execute(
            &area,
            Some(&profiles),
            &args(
                "fill",
                vec![
                    ("selector", Value::string("#entry")),
                    ("text", Value::string("hello")),
                ],
            ),
            &cancel,
        )
        .unwrap();
    let snapshot = registry
        .execute(&area, Some(&profiles), &args("snapshot", vec![]), &cancel)
        .unwrap();
    assert!(
        snapshot
            .get("elements")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .any(|e| e.get("ref") == Some(&Value::string("@2")))
    );
    registry
        .execute(
            &area,
            Some(&profiles),
            &args("click", vec![("selector", Value::string("@2"))]),
            &cancel,
        )
        .unwrap();
    let untouched = registry
        .execute(
            &area,
            Some(&profiles),
            &args(
                "evaluate",
                vec![(
                    "expression",
                    Value::string("document.querySelectorAll('[data-jecode-ref]').length"),
                )],
            ),
            &cancel,
        )
        .unwrap();
    assert_eq!(untouched.get("value"), Some(&Value::number(0)));
    registry
        .execute(
            &area,
            Some(&profiles),
            &args(
                "wait",
                vec![
                    ("selector", Value::string("#out")),
                    ("text", Value::string("hello")),
                ],
            ),
            &cancel,
        )
        .unwrap();
    let screenshot = registry
        .execute(
            &area,
            Some(&profiles),
            &args(
                "screenshot",
                vec![
                    ("width", Value::number(800)),
                    ("height", Value::number(600)),
                ],
            ),
            &cancel,
        )
        .unwrap();
    let relative = screenshot
        .get("screenshot_path")
        .and_then(Value::as_str)
        .unwrap()
        .strip_prefix("tmp:")
        .unwrap();
    assert!(
        std::fs::metadata(area.ensure().unwrap().join(relative))
            .unwrap()
            .len()
            > 100
    );
    registry
        .execute(
            &area,
            Some(&profiles),
            &Value::object([("action", Value::string("stop"))]),
            &cancel,
        )
        .unwrap();
    let reopen = registry
        .execute(
            &area,
            Some(&profiles),
            &Value::object([
                ("action", Value::string("open")),
                ("url", Value::string(&url)),
                ("profile", Value::string("fixture")),
            ]),
            &cancel,
        )
        .unwrap();
    let second = reopen.get("tab_id").and_then(Value::as_str).unwrap();
    let persisted = registry
        .execute(
            &area,
            Some(&profiles),
            &Value::object([
                ("action", Value::string("evaluate")),
                ("tab_id", Value::string(second)),
                ("expression", Value::string("localStorage.getItem('saved')")),
            ]),
            &cancel,
        )
        .unwrap();
    assert_eq!(persisted.get("value"), Some(&Value::string("hello")));
    registry.stop_all();
    running.store(false, Ordering::Relaxed);
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(fixture);
}
