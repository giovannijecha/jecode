use super::tests::fake_page;
use super::{Browser, actions, transport::Cdp};
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::scratch::Area;
use crate::test_support::Directory;
use std::time::{Duration, Instant};

fn browser(endpoint: &str) -> Browser {
    let mut browser = Browser {
        supervisor: None,
        lifetime: Cancellation::default(),
        profile: std::path::PathBuf::new(),
        lease: None,
        port: 0,
        tabs: std::collections::BTreeMap::new(),
        viewports: std::collections::BTreeMap::new(),
        id: "fixture".into(),
    };
    browser
        .tabs
        .insert("tab-1".into(), Cdp::connect(endpoint).unwrap());
    browser
}

fn wait(
    browser: &mut Browser,
    fields: Vec<(&str, Value)>,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    let directory = Directory::new();
    let area = Area::new(
        directory.path().join("123456-789"),
        directory.path().into(),
        "123456-789",
    )
    .unwrap();
    let mut arguments = Value::object([
        ("action", Value::string("wait")),
        ("tab_id", Value::string("tab-1")),
    ]);
    if let Value::Object(entries) = &mut arguments {
        for (key, value) in fields {
            entries.insert(key.into(), value);
        }
    }
    actions::execute(browser, &area, "wait", &arguments, cancellation)
}

/// A page whose condition check answers with `state` on every poll.
fn page(state: Value) -> (String, std::thread::JoinHandle<()>) {
    fake_page(move |method, _| {
        assert_eq!(method, "Runtime.evaluate");
        (
            Vec::new(),
            Some(Value::object([(
                "result",
                Value::object([("value", state.clone())]),
            )])),
        )
    })
}

fn state(found: bool, element: bool) -> Value {
    Value::object([
        ("found", Value::Bool(found)),
        ("element", Value::Bool(element)),
        ("url", Value::string("http://fixture.test/form")),
    ])
}

#[test]
fn missing_selector_reports_tab_url_condition_and_budget() {
    let (endpoint, server) = page(state(false, false));
    let mut browser = browser(&endpoint);
    let error = wait(
        &mut browser,
        vec![
            ("selector", Value::string("#missing")),
            ("timeout_ms", Value::number(300)),
        ],
        &Cancellation::default(),
    )
    .unwrap_err();
    assert!(
        error.starts_with("Browser wait timed out after 300 ms"),
        "{error}"
    );
    assert!(error.contains("elapsed "), "{error}");
    assert!(error.contains("tab tab-1"), "{error}");
    assert!(error.contains("http://fixture.test/form"), "{error}");
    assert!(
        error.contains("no element matched selector \"#missing\""),
        "{error}"
    );
    drop(browser);
    server.join().unwrap();
}

#[test]
fn missing_text_names_the_text_and_where_it_was_searched() {
    let (endpoint, server) = page(state(false, true));
    let mut browser = browser(&endpoint);
    let error = wait(
        &mut browser,
        vec![
            ("text", Value::string("Saved")),
            ("timeout_ms", Value::number(300)),
        ],
        &Cancellation::default(),
    )
    .unwrap_err();
    assert!(
        error.contains("text \"Saved\" not found in the page body"),
        "{error}"
    );
    assert!(error.contains("http://fixture.test/form"), "{error}");
    drop(browser);
    server.join().unwrap();
}

#[test]
fn missing_selector_keeps_the_requested_text_in_the_diagnostic() {
    let (endpoint, server) = page(state(false, false));
    let mut browser = browser(&endpoint);
    let error = wait(
        &mut browser,
        vec![
            ("selector", Value::string("#missing")),
            ("text", Value::string("Saved")),
            ("timeout_ms", Value::number(300)),
        ],
        &Cancellation::default(),
    )
    .unwrap_err();
    assert!(
        error.contains("no element matched selector \"#missing\""),
        "{error}"
    );
    assert!(error.contains("text \"Saved\""), "{error}");
    drop(browser);
    server.join().unwrap();
}

#[test]
fn unanswered_final_check_preserves_the_last_completed_observation() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    for (key, value, element, diagnosis) in [
        (
            "selector",
            "#missing",
            false,
            "no element matched selector \"#missing\"",
        ),
        (
            "text",
            "Saved",
            true,
            "text \"Saved\" not found in the page body",
        ),
    ] {
        let checks = Arc::new(AtomicUsize::new(0));
        let received = Arc::clone(&checks);
        let (endpoint, server) = fake_page(move |_, _| {
            let reply = (received.fetch_add(1, Ordering::SeqCst) == 0).then(|| {
                Value::object([("result", Value::object([("value", state(false, element))]))])
            });
            (Vec::new(), reply)
        });
        let mut browser = browser(&endpoint);
        let error = wait(
            &mut browser,
            vec![
                (key, Value::string(value)),
                ("timeout_ms", Value::number(250)),
            ],
            &Cancellation::default(),
        )
        .unwrap_err();
        assert!(
            checks.load(Ordering::SeqCst) >= 2,
            "the final check must go unanswered"
        );
        assert!(error.contains(diagnosis), "{error}");
        assert!(error.contains("last completed check"), "{error}");
        assert!(error.contains("did not answer"), "{error}");
        assert!(error.contains("http://fixture.test/form"), "{error}");
        drop(browser);
        server.join().unwrap();
    }
}

#[test]
fn successful_wait_keeps_its_result() {
    let (endpoint, server) = page(state(true, true));
    let mut browser = browser(&endpoint);
    let found = wait(
        &mut browser,
        vec![("selector", Value::string("#ready"))],
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(
        found,
        Value::object([
            ("found", Value::Bool(true)),
            ("tab_id", Value::string("tab-1")),
        ])
    );
    drop(browser);
    server.join().unwrap();
}

#[test]
fn short_timeout_bounds_an_unanswered_evaluation() {
    let (endpoint, server) = fake_page(|_, _| (Vec::new(), None));
    let mut browser = browser(&endpoint);
    let started = Instant::now();
    let error = wait(
        &mut browser,
        vec![
            ("text", Value::string("never")),
            ("timeout_ms", Value::number(300)),
        ],
        &Cancellation::default(),
    )
    .unwrap_err();
    // The former fixed two-second evaluation timeout would exceed this bound.
    assert!(started.elapsed() < Duration::from_millis(1500), "{error}");
    assert!(
        error.starts_with("Browser wait timed out after 300 ms"),
        "{error}"
    );
    assert!(error.contains("did not answer"), "{error}");
    assert!(error.contains("unknown URL"), "{error}");
    assert!(error.contains("text \"never\""), "{error}");
    drop(browser);
    server.join().unwrap();
}

#[test]
fn unanswered_wait_keeps_both_selector_and_text() {
    let (endpoint, server) = fake_page(|_, _| (Vec::new(), None));
    let mut browser = browser(&endpoint);
    let error = wait(
        &mut browser,
        vec![
            ("selector", Value::string("#result")),
            ("text", Value::string("Saved")),
            ("timeout_ms", Value::number(100)),
        ],
        &Cancellation::default(),
    )
    .unwrap_err();
    assert!(error.contains("selector \"#result\""), "{error}");
    assert!(error.contains("text \"Saved\""), "{error}");
    assert!(error.contains("did not answer"), "{error}");
    drop(browser);
    server.join().unwrap();
}

#[test]
fn reply_body_after_the_deadline_cannot_satisfy_wait() {
    use super::tests::{accept_websocket, frame, read_client_frame};
    use std::io::Write;
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("ws://{}/page", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut socket = accept_websocket(&listener);
        let _ = read_client_frame(&mut socket);
        let payload = Value::object([
            ("id", Value::number(1)),
            (
                "result",
                Value::object([("result", Value::object([("value", state(true, true))]))]),
            ),
        ])
        .encode();
        let packet = frame(1, true, payload.as_bytes());
        let header = packet.len() - payload.len();
        socket.write_all(&packet[..header]).unwrap();
        std::thread::sleep(Duration::from_millis(125));
        let _ = socket.write_all(&packet[header..]);
    });
    let mut browser = browser(&endpoint);
    let error = wait(
        &mut browser,
        vec![
            ("text", Value::string("Saved")),
            ("timeout_ms", Value::number(50)),
        ],
        &Cancellation::default(),
    )
    .unwrap_err();
    assert!(
        error.starts_with("Browser wait timed out after 50 ms"),
        "{error}"
    );
    assert!(error.contains("text \"Saved\""), "{error}");
    assert!(!browser.tabs.get("tab-1").unwrap().usable());
    drop(browser);
    server.join().unwrap();
}

#[test]
fn cancellation_stops_polling_promptly() {
    let (endpoint, server) = page(state(false, true));
    let mut browser = browser(&endpoint);
    let cancellation = Cancellation::default();
    let trigger = cancellation.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        trigger.cancel();
    });
    let started = Instant::now();
    let error = wait(
        &mut browser,
        vec![
            ("text", Value::string("never")),
            ("timeout_ms", Value::number(30000)),
        ],
        &cancellation,
    )
    .unwrap_err();
    assert_eq!(error, "Operation cancelled");
    assert!(started.elapsed() < Duration::from_secs(2));
    drop(browser);
    server.join().unwrap();
}

#[test]
fn javascript_errors_are_not_reported_as_timeouts() {
    let (endpoint, server) = fake_page(|_, _| {
        (
            Vec::new(),
            Some(Value::object([
                ("result", Value::object([("type", Value::string("object"))])),
                (
                    "exceptionDetails",
                    Value::object([
                        ("text", Value::string("Uncaught")),
                        (
                            "exception",
                            Value::object([(
                                "description",
                                Value::string("SyntaxError: '##' is not a valid selector"),
                            )]),
                        ),
                    ]),
                ),
            ])),
        )
    });
    let mut browser = browser(&endpoint);
    let error = wait(
        &mut browser,
        vec![
            ("selector", Value::string("##")),
            ("timeout_ms", Value::number(5000)),
        ],
        &Cancellation::default(),
    )
    .unwrap_err();
    assert!(error.starts_with("Browser JavaScript error: "), "{error}");
    drop(browser);
    server.join().unwrap();
}
