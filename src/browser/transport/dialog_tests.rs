use super::Cdp;
use crate::browser::tests::{accept_websocket, frame, read_client_frame, try_read_client_frame};
use crate::cancel::Cancellation;
use crate::json::{self, Value};
use std::io::Write;
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

fn event(method: &str) -> Vec<u8> {
    frame(
        1,
        true,
        Value::object([
            ("method", Value::string(method)),
            (
                "params",
                Value::object([
                    ("type", Value::string("alert")),
                    ("message", Value::string("Hi")),
                ]),
            ),
        ])
        .encode()
        .as_bytes(),
    )
}

#[test]
fn manual_dialog_close_is_received_before_the_next_normal_call() {
    external_close(false);
}

#[test]
fn a_fragmented_dialog_close_preserves_protocol_alignment() {
    external_close(true);
}

fn external_close(fragmented: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "ws://127.0.0.1:{}/devtools/page/dialog",
        listener.local_addr().unwrap().port()
    );
    let (close, closing) = mpsc::channel();
    let (ready, queued) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let mut stream = accept_websocket(&listener);
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        read_client_frame(&mut stream);
        stream
            .write_all(&event("Page.javascriptDialogOpening"))
            .unwrap();
        closing.recv_timeout(Duration::from_secs(3)).unwrap();
        let closed = event("Page.javascriptDialogClosed");
        let split = if fragmented { 3 } else { closed.len() };
        stream.write_all(&closed[..split]).unwrap();
        ready.send(()).unwrap();
        // The probe goes out while the closing frame is only partly received.
        let Some((_, request)) = try_read_client_frame(&mut stream) else {
            return;
        };
        stream.write_all(&closed[split..]).unwrap();
        // The request interrupted by the dialog replies after the person closes it.
        stream
            .write_all(&frame(1, true, br#"{"id":1,"result":{}}"#))
            .unwrap();
        let request = json::parse(&String::from_utf8(request).unwrap()).unwrap();
        assert_eq!(
            request.get("method"),
            Some(&Value::string("Browser.getVersion"))
        );
        let reply = Value::object([
            ("id", request.get("id").unwrap().clone()),
            ("result", Value::object([])),
        ]);
        stream
            .write_all(&frame(1, true, reply.encode().as_bytes()))
            .unwrap();
        while let Some((_, request)) = try_read_client_frame(&mut stream) {
            let request = json::parse(&String::from_utf8(request).unwrap()).unwrap();
            let reply = Value::object([
                ("id", request.get("id").unwrap().clone()),
                ("result", Value::object([])),
            ]);
            stream
                .write_all(&frame(1, true, reply.encode().as_bytes()))
                .unwrap();
            if request.get("method").and_then(Value::as_str) == Some("Runtime.evaluate") {
                break;
            }
        }
    });
    let mut connection = Cdp::connect(&url).unwrap();
    let cancellation = Cancellation::default();
    assert!(
        connection
            .call(
                "Runtime.evaluate",
                Value::object([]),
                &cancellation,
                Duration::from_secs(2)
            )
            .unwrap_err()
            .contains("alert")
    );
    close.send(()).unwrap();
    queued.recv_timeout(Duration::from_secs(3)).unwrap();
    let resumed = connection.call(
        "Runtime.evaluate",
        Value::object([]),
        &cancellation,
        Duration::from_secs(2),
    );
    drop(connection);
    server.join().unwrap();
    assert!(resumed.is_ok(), "{resumed:?}");
}

#[test]
fn a_new_dialog_before_the_previous_answer_reply_stays_pending() {
    use std::sync::{Arc, Mutex};
    let requests = Arc::new(Mutex::new(0));
    let (url, server) = crate::browser::tests::fake_page(move |_, _| {
        let mut count = requests.lock().unwrap();
        *count += 1;
        let opening = |message| {
            Value::object([
                ("method", Value::string("Page.javascriptDialogOpening")),
                (
                    "params",
                    Value::object([
                        ("type", Value::string("confirm")),
                        ("message", Value::string(message)),
                    ]),
                ),
            ])
        };
        let closed = || Value::object([("method", Value::string("Page.javascriptDialogClosed"))]);
        match *count {
            1 => (vec![opening("First")], None),
            2 => (vec![closed(), opening("Second")], Some(Value::object([]))),
            4 => (vec![closed()], Some(Value::object([]))),
            _ => (vec![], Some(Value::object([]))),
        }
    });
    let mut connection = Cdp::connect(&url).unwrap();
    let mut call = |method| {
        connection.call(
            method,
            Value::object([]),
            &Cancellation::default(),
            Duration::from_secs(2),
        )
    };
    assert!(call("Runtime.evaluate").unwrap_err().contains("First"));
    call("Page.handleJavaScriptDialog").unwrap();
    assert!(call("Runtime.evaluate").unwrap_err().contains("Second"));
    call("Page.handleJavaScriptDialog").unwrap();
    call("Runtime.evaluate").unwrap();
    drop(connection);
    server.join().unwrap();
}
