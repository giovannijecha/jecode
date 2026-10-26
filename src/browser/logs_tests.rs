use super::logs::{EventLog, Filter};
use super::tests::fake_page;
use super::{Browser, actions, transport::Cdp};
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::scratch::Area;
use crate::test_support::Directory;

fn event(method: &str, params: Value) -> Value {
    Value::object([("method", Value::string(method)), ("params", params)])
}

fn console(kind: &str, message: &str) -> Value {
    event(
        "Runtime.consoleAPICalled",
        Value::object([
            ("type", Value::string(kind)),
            (
                "args",
                Value::Array(vec![
                    Value::object([("value", Value::string(message))]),
                    Value::object([("value", Value::number(7))]),
                ]),
            ),
        ]),
    )
}

fn request(id: &str, url: &str) -> Value {
    event(
        "Network.requestWillBeSent",
        Value::object([
            ("requestId", Value::string(id)),
            (
                "request",
                Value::object([
                    ("method", Value::string("GET")),
                    ("url", Value::string(url)),
                ]),
            ),
        ]),
    )
}

/// Console, exception, network and navigation traffic in a fixed order.
fn mixed() -> EventLog {
    let mut log = EventLog::default();
    log.push(request("r1", "http://site.test/"));
    log.push(console("log", "ready"));
    log.push(event(
        "Page.frameNavigated",
        Value::object([(
            "frame",
            Value::object([("url", Value::string("http://site.test/"))]),
        )]),
    ));
    log.push(event(
        "Runtime.exceptionThrown",
        Value::object([(
            "exceptionDetails",
            Value::object([
                ("text", Value::string("Uncaught")),
                ("url", Value::string("http://site.test/app.js")),
                ("lineNumber", Value::number(4)),
                ("columnNumber", Value::number(2)),
                (
                    "exception",
                    Value::object([(
                        "description",
                        Value::string("Error: boom\n    at app.js:5:3"),
                    )]),
                ),
            ]),
        )]),
    ));
    log.push(request("r2", "http://site.test/missing.png"));
    log.push(event(
        "Network.responseReceived",
        Value::object([(
            "response",
            Value::object([
                ("status", Value::number(404)),
                ("url", Value::string("http://site.test/missing.png")),
            ]),
        )]),
    ));
    log.push(request("r3", "http://down.test/api"));
    log.push(event(
        "Network.loadingFailed",
        Value::object([
            ("requestId", Value::string("r3")),
            ("errorText", Value::string("net::ERR_CONNECTION_REFUSED")),
        ]),
    ));
    log.push(console("warning", "slow"));
    log
}

fn texts(entries: &[Value]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| {
            format!(
                "{} {} {} {}",
                entry.get("seq").and_then(Value::as_usize).unwrap(),
                entry.get("kind").and_then(Value::as_str).unwrap(),
                entry.get("level").and_then(Value::as_str).unwrap(),
                entry.get("text").and_then(Value::as_str).unwrap()
            )
        })
        .collect()
}

#[test]
fn default_filter_keeps_console_exceptions_and_failures_only() {
    let page = mixed().page(Filter::Default, 50, None).unwrap();
    assert_eq!(
        texts(&page.entries),
        [
            "1 console log ready 7",
            "3 exception error Error: boom",
            "5 network error 404 http://site.test/missing.png",
            "7 network error net::ERR_CONNECTION_REFUSED http://down.test/api",
            "8 console warning slow 7",
        ]
    );
    assert_eq!(
        page.entries[1].get("source"),
        Some(&Value::string("http://site.test/app.js:5:3"))
    );
    assert_eq!((page.more, page.missed), (false, 0));
    assert_eq!(page.records.len(), 9, "every examined event is saved");
    assert!(
        page.records[0]
            .get("event")
            .and_then(|raw| raw.get("method"))
            .is_some()
    );
}

#[test]
fn filters_select_console_errors_network_and_all() {
    let log = mixed();
    let count = |filter, name: &str| {
        Filter::parse(Some(name))
            .ok()
            .filter(|parsed| *parsed == filter)
            .unwrap();
        log.page(filter, 50, None).unwrap().entries.len()
    };
    assert_eq!(count(Filter::Console, "console"), 2);
    assert_eq!(count(Filter::Errors, "errors"), 3);
    assert_eq!(count(Filter::Network, "network"), 5);
    assert_eq!(count(Filter::All, "all"), 9);
    assert!(Filter::parse(Some("verbose")).is_err());
}

#[test]
fn failed_redirect_uses_the_last_request_url() {
    let mut log = EventLog::default();
    log.push(request("redirect", "http://site.test/start"));
    log.push(request("redirect", "http://site.test/final"));
    log.push(event(
        "Network.loadingFailed",
        Value::object([
            ("requestId", Value::string("redirect")),
            ("errorText", Value::string("net::ERR_CONNECTION_REFUSED")),
        ]),
    ));
    let page = log.page(Filter::Errors, 50, None).unwrap();
    assert_eq!(
        texts(&page.entries),
        ["2 network error net::ERR_CONNECTION_REFUSED http://site.test/final"]
    );
}

#[test]
fn consecutive_pages_are_disjoint_complete_and_then_empty() {
    let mut log = mixed();
    let mut cursor = None;
    let mut seen = Vec::new();
    loop {
        let page = log.page(Filter::All, 4, cursor.as_deref()).unwrap();
        assert_eq!(page.missed, 0);
        seen.extend(texts(&page.entries));
        cursor = Some(page.next_cursor.clone());
        if !page.more {
            break;
        }
    }
    let sequences: Vec<_> = seen
        .iter()
        .map(|line| line.split(' ').next().unwrap().parse::<u64>().unwrap())
        .collect();
    assert_eq!(sequences, (0..9).collect::<Vec<_>>());
    let empty = log.page(Filter::All, 4, cursor.as_deref()).unwrap();
    assert!(empty.entries.is_empty() && empty.records.is_empty() && !empty.more);
    assert_eq!(Some(empty.next_cursor.clone()), cursor);
    log.push(console("error", "after read"));
    let later = log.page(Filter::Default, 4, cursor.as_deref()).unwrap();
    assert_eq!(texts(&later.entries), ["9 console error after read 7"]);
}

#[test]
fn limited_filtered_pages_resume_after_the_last_returned_event() {
    let log = mixed();
    let first = log.page(Filter::Default, 2, None).unwrap();
    assert_eq!(texts(&first.entries).len(), 2);
    assert!(first.more);
    assert_eq!(
        first.records.len(),
        4,
        "records stop at the last returned entry"
    );
    let second = log
        .page(Filter::Default, 2, Some(&first.next_cursor))
        .unwrap();
    assert_eq!(
        texts(&second.entries),
        [
            "5 network error 404 http://site.test/missing.png",
            "7 network error net::ERR_CONNECTION_REFUSED http://down.test/api",
        ]
    );
    assert_eq!(second.records.len(), 4);
}

#[test]
fn nonmatching_raw_events_are_paged_by_count_without_loss() {
    let mut log = EventLog::default();
    for n in 0..205 {
        log.push(console("log", &format!("event {n}")));
    }
    let first = log.page(Filter::Network, 1, None).unwrap();
    assert!(first.entries.is_empty() && first.more);
    assert_eq!(first.records.len(), 200);
    let second = log
        .page(Filter::Network, 1, Some(&first.next_cursor))
        .unwrap();
    assert!(second.entries.is_empty() && !second.more);
    assert_eq!(second.records.len(), 5);
    let sequences: Vec<_> = first
        .records
        .iter()
        .chain(&second.records)
        .map(|record| record.get("seq").and_then(Value::as_usize).unwrap())
        .collect();
    assert_eq!(sequences, (0..205).collect::<Vec<_>>());
    assert_eq!((first.missed, second.missed), (0, 0));
}

#[test]
fn nonmatching_raw_events_are_paged_by_bytes_without_loss() {
    let mut log = EventLog::default();
    let message = "x".repeat(100_000);
    for _ in 0..3 {
        log.push(console("log", &message));
    }
    let first = log.page(Filter::Network, 1, None).unwrap();
    assert!(first.entries.is_empty() && first.more);
    assert_eq!(first.records.len(), 2);
    let second = log
        .page(Filter::Network, 1, Some(&first.next_cursor))
        .unwrap();
    assert!(second.entries.is_empty() && !second.more);
    assert_eq!(second.records.len(), 1);
    let sequences: Vec<_> = first
        .records
        .iter()
        .chain(&second.records)
        .map(|record| record.get("seq").and_then(Value::as_usize).unwrap())
        .collect();
    assert_eq!(sequences, [0, 1, 2]);
}

#[test]
fn oversized_raw_event_alone_still_advances_the_cursor() {
    let mut log = EventLog::default();
    log.push(console("log", &"x".repeat(300_000)));
    log.push(console("log", "later"));
    let first = log.page(Filter::Network, 1, None).unwrap();
    assert_eq!(first.records.len(), 1);
    assert!(first.more);
    let second = log
        .page(Filter::Network, 1, Some(&first.next_cursor))
        .unwrap();
    assert_eq!(second.records[0].get("seq"), Some(&Value::number(1)));
    assert!(!second.more);
}

#[test]
fn eviction_is_reported_as_missed_history() {
    let mut log = EventLog::with_capacity(5);
    let early = log.page(Filter::All, 50, None).unwrap().next_cursor;
    for n in 0..12 {
        log.push(console("log", &format!("event {n}")));
    }
    let fresh = log.page(Filter::All, 50, None).unwrap();
    assert_eq!(fresh.missed, 7);
    assert_eq!(
        texts(&fresh.entries).first().unwrap(),
        "7 console log event 7 7"
    );
    let resumed = log.page(Filter::All, 2, Some(&early)).unwrap();
    assert_eq!(resumed.missed, 7);
    let after = log
        .page(Filter::All, 50, Some(&resumed.next_cursor))
        .unwrap();
    assert_eq!(after.missed, 0, "a retained cursor misses nothing");
    assert_eq!(after.entries.len(), 3);
}

#[test]
fn malformed_foreign_and_future_cursors_are_rejected() {
    let log = mixed();
    let other = EventLog::default().page(Filter::All, 1, None).unwrap();
    let id = log.page(Filter::All, 1, None).unwrap().next_cursor;
    let (prefix, _) = id.split_once('.').unwrap();
    for cursor in [
        "",
        "abc",
        "1",
        "1.x",
        &other.next_cursor,
        &format!("{prefix}.99"),
    ] {
        assert!(log.page(Filter::All, 1, Some(cursor)).is_err(), "{cursor}");
    }
    assert!(
        log.page(Filter::All, 1, Some(&other.next_cursor))
            .err()
            .is_some_and(|error| error.contains("omit cursor"))
    );
}

#[test]
fn cursor_child_process() {
    let Ok(mode) = std::env::var("JECODE_CURSOR_CHILD") else {
        return;
    };
    let mut log = EventLog::default();
    match mode.as_str() {
        "emit" => println!(
            "JECODE_CURSOR={}",
            log.page(Filter::All, 1, None).unwrap().next_cursor
        ),
        "check" => {
            log.push(console("log", "new session"));
            let cursor = std::env::var("JECODE_OLD_CURSOR").unwrap();
            assert!(log.page(Filter::All, 1, Some(&cursor)).is_err());
        }
        _ => panic!("Unknown cursor child mode"),
    }
}

#[test]
fn cursor_from_another_process_is_rejected() {
    let exe = std::env::current_exe().unwrap();
    let child = |mode: &str, cursor: Option<&str>| {
        let mut command = std::process::Command::new(&exe);
        command
            .arg("cursor_child_process")
            .arg("--nocapture")
            .env("JECODE_CURSOR_CHILD", mode);
        if let Some(cursor) = cursor {
            command.env("JECODE_OLD_CURSOR", cursor);
        }
        command.output().unwrap()
    };
    let emitted = child("emit", None);
    assert!(emitted.status.success());
    let output = String::from_utf8(emitted.stdout).unwrap();
    let cursor = output
        .lines()
        .find_map(|line| line.split_once("JECODE_CURSOR=").map(|(_, value)| value))
        .unwrap();
    let checked = child("check", Some(cursor));
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stdout)
    );
}

#[test]
fn gaps_appear_under_every_filter() {
    let mut log = EventLog::default();
    log.gap("DevTools connection was recreated");
    for filter in [
        Filter::Default,
        Filter::Console,
        Filter::Errors,
        Filter::Network,
    ] {
        let page = log.page(filter, 5, None).unwrap();
        assert_eq!(
            texts(&page.entries),
            ["0 gap warning DevTools connection was recreated"]
        );
    }
}

#[test]
fn logs_action_pages_compact_entries_and_saves_raw_records() {
    let (endpoint, server) = fake_page(|_, _| {
        (
            vec![
                console("log", "tick"),
                request("r", "http://site.test/poll"),
            ],
            Some(Value::object([("result", Value::object([]))])),
        )
    });
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
        .insert("tab-1".into(), Cdp::connect(&endpoint).unwrap());
    let directory = Directory::new();
    let area = Area::new(
        directory.path().join("123456-789"),
        directory.path().into(),
        "123456-789",
    )
    .unwrap();
    let logs = |fields: Vec<(&str, Value)>| {
        let mut arguments = Value::object([("tab_id", Value::string("tab-1"))]);
        if let Value::Object(entries) = &mut arguments {
            for (key, value) in fields {
                entries.insert(key.into(), value);
            }
        }
        arguments
    };
    let cancel = Cancellation::default();
    let first = actions::execute(&mut browser, &area, "logs", &logs(vec![]), &cancel).unwrap();
    let entries = first.get("entries").and_then(Value::as_array).unwrap();
    assert_eq!(texts(entries), ["0 console log tick 7"]);
    assert_eq!(first.get("missed"), Some(&Value::number(0)));
    let saved = first
        .get("logs_path")
        .and_then(Value::as_str)
        .unwrap()
        .strip_prefix("tmp:")
        .unwrap();
    let artifact = std::fs::read_to_string(area.ensure().unwrap().join(saved)).unwrap();
    let lines: Vec<_> = artifact.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[1].contains("\"method\":\"Network.requestWillBeSent\""));
    let cursor = first.get("next_cursor").cloned().unwrap();
    let second = actions::execute(
        &mut browser,
        &area,
        "logs",
        &logs(vec![("cursor", cursor)]),
        &cancel,
    )
    .unwrap();
    let entries = second.get("entries").and_then(Value::as_array).unwrap();
    assert_eq!(texts(entries), ["2 console log tick 7"]);
    for (key, value) in [
        ("limit", Value::number(0)),
        ("limit", Value::number(201)),
        ("filter", Value::string("debug")),
        ("cursor", Value::string("bogus")),
    ] {
        assert!(
            actions::execute(
                &mut browser,
                &area,
                "logs",
                &logs(vec![(key, value)]),
                &cancel
            )
            .is_err()
        );
    }
    drop(browser);
    server.join().unwrap();
}
