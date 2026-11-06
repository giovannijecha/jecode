use super::tests::{accept_websocket, fake_page, frame, read_client_frame};
use super::{keyboard, transport::Cdp};
use crate::cancel::Cancellation;
use crate::json::{self, Value};
use std::io::Write;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Calls = Arc<Mutex<Vec<(String, Value)>>>;

fn recording() -> (Cdp, Calls, std::thread::JoinHandle<()>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let (url, server) = fake_page(move |method, params| {
        recorded
            .lock()
            .unwrap()
            .push((method.into(), params.clone()));
        let result = if method == "Runtime.evaluate" {
            Value::object([("result", Value::object([("value", Value::Bool(true))]))])
        } else {
            Value::object([])
        };
        (vec![], Some(result))
    });
    (Cdp::connect(&url).unwrap(), calls, server)
}

#[test]
fn press_chords_and_masks_deliver_shortcuts_without_inserting_text() {
    let (mut connection, calls, server) = recording();
    for arguments in [
        r#"{"key":"Ctrl+Z"}"#,
        r#"{"key":"z","modifiers":10}"#,
        r#"{"key":"Meta+Shift+z"}"#,
        r#"{"key":"Alt+ArrowLeft"}"#,
        r#"{"key":"Shift+Tab"}"#,
    ] {
        keyboard::run(
            &mut connection,
            "press",
            &json::parse(arguments).unwrap(),
            &Cancellation::default(),
            Duration::from_secs(2),
        )
        .unwrap();
    }
    drop(connection);
    server.join().unwrap();
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 10);
    for (pair, (key, code, vk, modifiers)) in calls.chunks_exact(2).zip([
        ("z", "KeyZ", 90, 2),
        ("Z", "KeyZ", 90, 10),
        ("Z", "KeyZ", 90, 12),
        ("ArrowLeft", "ArrowLeft", 37, 1),
        ("Tab", "Tab", 9, 8),
    ]) {
        for ((method, event), kind) in pair.iter().zip(["keyDown", "keyUp"]) {
            assert_eq!(method, "Input.dispatchKeyEvent");
            assert_eq!(event.get("type"), Some(&Value::string(kind)));
            assert_eq!(event.get("key"), Some(&Value::string(key)));
            assert_eq!(event.get("code"), Some(&Value::string(code)));
            assert_eq!(event.get("windowsVirtualKeyCode"), Some(&Value::number(vk)));
            assert_eq!(event.get("modifiers"), Some(&Value::number(modifiers)));
            assert!(event.get("text").is_none());
        }
    }
}

#[test]
fn press_names_ignore_case_and_accept_common_aliases() {
    let (mut connection, calls, server) = recording();
    for arguments in [
        r#"{"key":"enter"}"#,
        r#"{"key":"ctrl+a"}"#,
        r#"{"key":"CTRL+SHIFT+TAB"}"#,
        r#"{"key":"Option+Left"}"#,
        r#"{"key":"esc"}"#,
        r#"{"key":"DEL"}"#,
        r#"{"key":"f5"}"#,
        r#"{"key":"Cmd+spacebar"}"#,
        // Absent optional fields may arrive as explicit nulls.
        r#"{"key":"up","modifiers":null,"selector":null}"#,
    ] {
        keyboard::run(
            &mut connection,
            "press",
            &json::parse(arguments).unwrap(),
            &Cancellation::default(),
            Duration::from_secs(2),
        )
        .unwrap();
    }
    drop(connection);
    server.join().unwrap();
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 18);
    for (pair, (key, code, vk, modifiers)) in calls.chunks_exact(2).zip([
        ("Enter", "Enter", 13, 0),
        ("a", "KeyA", 65, 2),
        ("Tab", "Tab", 9, 10),
        ("ArrowLeft", "ArrowLeft", 37, 1),
        ("Escape", "Escape", 27, 0),
        ("Delete", "Delete", 46, 0),
        ("F5", "F5", 116, 0),
        (" ", "Space", 32, 4),
        ("ArrowUp", "ArrowUp", 38, 0),
    ]) {
        let event = &pair[0].1;
        assert_eq!(event.get("key"), Some(&Value::string(key)));
        assert_eq!(event.get("code"), Some(&Value::string(code)));
        assert_eq!(event.get("windowsVirtualKeyCode"), Some(&Value::number(vk)));
        assert_eq!(event.get("modifiers"), Some(&Value::number(modifiers)));
    }
}

#[test]
fn unknown_names_list_what_the_model_can_use() {
    let (mut connection, _, server) = recording();
    let press = |connection: &mut Cdp, key: &str| {
        keyboard::run(
            connection,
            "press",
            &Value::object([("key", Value::string(key))]),
            &Cancellation::default(),
            Duration::from_secs(2),
        )
        .unwrap_err()
    };
    let error = press(&mut connection, "Hyper+a");
    assert!(error.contains("Hyper") && error.contains("Alt, Ctrl, Meta or Shift"));
    let error = press(&mut connection, "NumLock");
    assert!(error.contains("NumLock") && error.contains("ArrowLeft") && error.contains("F1-F24"));
    drop(connection);
    server.join().unwrap();
}

#[test]
fn a_stopped_sequence_reports_the_keys_that_completed() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let requests = AtomicUsize::new(0);
    let (url, server) = fake_page(move |_, _| {
        let reply = requests.fetch_add(1, Ordering::Relaxed) < 4;
        (vec![], reply.then(|| Value::object([])))
    });
    let mut connection = Cdp::connect(&url).unwrap();
    let error = keyboard::run(
        &mut connection,
        "type",
        &Value::object([("text", Value::string("abc"))]),
        &Cancellation::default(),
        Duration::from_millis(300),
    )
    .unwrap_err();
    assert!(
        error.starts_with("2 of 3 keys completed: ") && error.contains("timed out"),
        "{error}"
    );
    drop(connection);
    server.join().unwrap();
}

#[test]
fn a_dialog_stops_input_at_once_and_blocks_calls_until_answered() {
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let (url, server) = fake_page(move |method, params| {
        let mut calls = recorded.lock().unwrap();
        calls.push((method.into(), params.clone()));
        // The second key's keyDown opens a dialog and is never acknowledged.
        if calls.len() == 3 {
            let opening = json::parse(
                r#"{"method":"Page.javascriptDialogOpening","params":{"type":"confirm","message":"Sure?"}}"#,
            )
            .unwrap();
            return (vec![opening], None);
        }
        if method == "Page.handleJavaScriptDialog" {
            return (
                vec![Value::object([(
                    "method",
                    Value::string("Page.javascriptDialogClosed"),
                )])],
                Some(Value::object([])),
            );
        }
        (vec![], Some(Value::object([])))
    });
    let mut connection = Cdp::connect(&url).unwrap();
    let cancellation = Cancellation::default();
    let started = std::time::Instant::now();
    let error = keyboard::run(
        &mut connection,
        "type",
        &Value::object([("text", Value::string("abc"))]),
        &cancellation,
        Duration::from_secs(30),
    )
    .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5), "{error}");
    assert!(error.starts_with("1 of 3 keys completed: "), "{error}");
    assert!(error.contains("confirm") && error.contains("\"Sure?\""));
    assert!(error.contains("Page.handleJavaScriptDialog"));
    // A browser-side probe refreshes events without dispatching renderer input
    // or losing the connection needed to answer the dialog.
    let refused = connection
        .call(
            "Runtime.evaluate",
            Value::object([]),
            &cancellation,
            Duration::from_secs(2),
        )
        .unwrap_err();
    assert!(refused.contains("\"Sure?\""));
    assert!(connection.usable());
    assert_eq!(calls.lock().unwrap().len(), 4);
    assert_eq!(calls.lock().unwrap()[3].0, "Browser.getVersion");
    connection
        .call(
            "Page.handleJavaScriptDialog",
            Value::object([("accept", Value::Bool(true))]),
            &cancellation,
            Duration::from_secs(2),
        )
        .unwrap();
    connection
        .call(
            "Runtime.evaluate",
            Value::object([]),
            &cancellation,
            Duration::from_secs(2),
        )
        .unwrap();
    drop(connection);
    server.join().unwrap();
    let calls = calls.lock().unwrap();
    assert_eq!(calls[4].0, "Page.handleJavaScriptDialog");
    assert_eq!(calls[5].0, "Runtime.evaluate");
}

#[test]
fn a_dialog_closed_in_its_window_stops_blocking_calls() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let mut stream = accept_websocket(&listener);
        let event = |method: &str| {
            let event =
                format!(r#"{{"method":"{method}","params":{{"type":"alert","message":"Hi"}}}}"#);
            frame(1, true, event.as_bytes())
        };
        let request_id = |stream: &mut std::net::TcpStream| {
            let (_, request) = read_client_frame(stream);
            let request = json::parse(&String::from_utf8(request).unwrap()).unwrap();
            request.get("id").unwrap().encode()
        };
        request_id(&mut stream);
        stream
            .write_all(&event("Page.javascriptDialogOpening"))
            .unwrap();
        // The person already answered: the closing event precedes the error.
        let id = request_id(&mut stream);
        let reply = format!(r#"{{"id":{id},"error":{{"message":"No dialog is showing"}}}}"#);
        stream
            .write_all(&event("Page.javascriptDialogClosed"))
            .unwrap();
        stream.write_all(&frame(1, true, reply.as_bytes())).unwrap();
        let id = request_id(&mut stream);
        let reply = format!(r#"{{"id":{id},"result":{{}}}}"#);
        stream.write_all(&frame(1, true, reply.as_bytes())).unwrap();
    });
    let mut connection = Cdp::connect(&format!("ws://127.0.0.1:{port}/devtools/page/x")).unwrap();
    let cancellation = Cancellation::default();
    let mut call = |method: &str| {
        connection.call(
            method,
            Value::object([]),
            &cancellation,
            Duration::from_secs(2),
        )
    };
    assert!(
        call("Input.dispatchKeyEvent")
            .unwrap_err()
            .contains("alert")
    );
    assert!(
        call("Page.handleJavaScriptDialog")
            .unwrap_err()
            .contains("No dialog is showing")
    );
    call("Runtime.evaluate").unwrap();
    server.join().unwrap();
}

#[test]
fn type_preserves_unicode_normalizes_crlf_and_focuses_only_once() {
    let (mut connection, calls, server) = recording();
    keyboard::run(
        &mut connection,
        "type",
        &Value::object([
            ("text", Value::string("A6+é🦀\r\nb\tc")),
            ("selector", Value::string("@2")),
        ]),
        &Cancellation::default(),
        Duration::from_secs(2),
    )
    .unwrap();
    drop(connection);
    server.join().unwrap();
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 19);
    assert_eq!(calls[0].0, "Runtime.evaluate");
    assert!(
        calls[0]
            .1
            .get("expression")
            .and_then(Value::as_str)
            .unwrap()
            .contains("refs')]?.[1]")
    );
    let downs: Vec<&Value> = calls[1..]
        .iter()
        .step_by(2)
        .map(|(_, event)| event)
        .collect();
    assert_eq!(
        downs
            .iter()
            .filter_map(|event| event.get("text").and_then(Value::as_str))
            .collect::<String>(),
        "A6+é🦀\rbc"
    );
    assert_eq!(downs[2].get("code"), Some(&Value::string("Equal")));
    assert_eq!(downs[2].get("modifiers"), Some(&Value::number(8)));
    assert_eq!(downs[3].get("code"), Some(&Value::string("")));
    assert_eq!(downs[7].get("key"), Some(&Value::string("Tab")));
    for (_, event) in calls[2..].iter().step_by(2) {
        assert_eq!(event.get("type"), Some(&Value::string("keyUp")));
        assert!(event.get("text").is_none());
    }
}

#[test]
fn invalid_keyboard_requests_send_no_events_or_focus_changes() {
    let (mut connection, calls, server) = recording();
    for (action, request) in [
        ("press", r#"{"key":"a","modifiers":16}"#),
        ("press", r#"{"key":"a","modifiers":-1}"#),
        ("press", r#"{"key":"a","modifiers":1.5}"#),
        ("press", r#"{"key":"a","modifiers":"2"}"#),
        ("press", r#"{"key":""}"#),
        ("press", r#"{"key":"Hyper+a"}"#),
        ("press", r#"{"key":"UnknownKey"}"#),
        ("press", r#"{}"#),
        (
            "type",
            r##"{"text":"before\u0000after","selector":"#editor"}"##,
        ),
        ("type", r#"{"text":42}"#),
    ] {
        assert!(
            keyboard::run(
                &mut connection,
                action,
                &json::parse(request).unwrap(),
                &Cancellation::default(),
                Duration::from_secs(2)
            )
            .is_err(),
            "{request}"
        );
    }
    for timeout in [Duration::ZERO, Duration::MAX] {
        let error = keyboard::run(
            &mut connection,
            "type",
            &Value::object([("text", Value::string("abc"))]),
            &Cancellation::default(),
            timeout,
        )
        .unwrap_err();
        assert!(error.contains("timed out") || error.contains("timeout is too large"));
    }
    drop(connection);
    server.join().unwrap();
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn cancelled_keyboard_input_stops_without_sending_a_key() {
    let (mut connection, calls, server) = recording();
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let error = keyboard::run(
        &mut connection,
        "type",
        &Value::object([("text", Value::string("abc"))]),
        &cancellation,
        Duration::from_secs(2),
    )
    .unwrap_err();
    assert_eq!(error, "Operation cancelled");
    drop(connection);
    server.join().unwrap();
    assert!(calls.lock().unwrap().is_empty());
}
