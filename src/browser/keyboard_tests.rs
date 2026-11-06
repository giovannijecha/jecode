use super::{keyboard, tests::fake_page, transport::Cdp};
use crate::cancel::Cancellation;
use crate::json::{self, Value};
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
