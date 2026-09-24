//! A saturated presentation channel must not control a launched process.
use super::*;
use std::{
    fs,
    sync::mpsc,
    time::{Duration, Instant},
};

fn backpressured_start(cancel: bool) {
    let files = Files::new();
    let workspace = crate::workspace::Workspace::open(&files.0).unwrap();
    let (events, received) = mpsc::sync_channel(64);
    for _ in 0..63 {
        events.send(Event::Thinking).unwrap();
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let stopped = Arc::new(AtomicBool::new(false));
    let context = worker::Context {
        events,
        cancelled: cancelled.clone(),
        stopped: stopped.clone(),
        guidance: Arc::new(queue::Pending::default()),
        next_effect: std::sync::atomic::AtomicU64::new(1),
        effect_gate: None,
    };
    let (done, completed) = mpsc::sync_channel(1);
    let handle = std::thread::spawn(move || {
        let (output, _) = super::super::command::execute(
            &native::script("backpressure-tree"),
            ".",
            if cancel { 20 } else { 8 },
            &crate::command::Shell::default(),
            &workspace,
            &context,
        );
        let _ = done.send(output);
    });
    let start_deadline = Instant::now() + Duration::from_secs(10);
    let paths = [
        files.0.join("backpressure-started"),
        files.0.join("backpressure-parent"),
        files.0.join("backpressure-descendant"),
        files.0.join("backpressure-tick"),
    ];
    let pids = loop {
        let parent = fs::read_to_string(&paths[1])
            .ok()
            .and_then(|text| text.parse::<u32>().ok());
        let descendant = fs::read_to_string(&paths[2])
            .ok()
            .and_then(|text| text.parse::<u32>().ok());
        if paths[0].exists()
            && paths[3].exists()
            && let Some(pair) = parent.zip(descendant)
        {
            break Some(pair);
        }
        if Instant::now() >= start_deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let started = pids.is_some();
    let mut stopped_before_release = false;
    let mut ticks_stopped = false;
    if let Some((parent, descendant)) = pids {
        if cancel {
            cancelled.store(true, Ordering::Release);
        }
        let stop_deadline = Instant::now() + Duration::from_secs(if cancel { 3 } else { 10 });
        while Instant::now() < stop_deadline {
            if native::stopped(parent) && native::stopped(descendant) {
                stopped_before_release = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let first = fs::read(&paths[3]).ok();
        std::thread::sleep(Duration::from_millis(150));
        ticks_stopped = first.is_some() && first == fs::read(&paths[3]).ok();
    }
    let before_release = completed.recv_timeout(Duration::from_secs(3)).ok();
    let receipt_ready_before_release = before_release.is_some();
    // Close the full presentation channel and join every test process before asserting.
    cancelled.store(true, Ordering::Release);
    stopped.store(true, Ordering::Release);
    drop(received);
    let outcome = before_release.or_else(|| completed.recv_timeout(Duration::from_secs(10)).ok());
    handle.join().unwrap();
    if let Some((parent, descendant)) = pids {
        native::assert_stopped(parent);
        native::assert_stopped(descendant);
    }
    let output = outcome.expect("command did not finish after backpressure was released");
    assert!(started, "fixture did not start: {}", output.summary);
    assert!(
        stopped_before_release,
        "process tree ran past cancellation or timeout"
    );
    assert!(ticks_stopped, "descendant kept changing the isolated file");
    assert!(
        receipt_ready_before_release,
        "the exact receipt waited for presentation backpressure"
    );
    let receipt = json::parse(&output.text, Default::default()).unwrap();
    assert_eq!(receipt.get("executed"), Some(&Value::Bool(true)));
    assert_eq!(
        receipt.get("status").and_then(Value::text),
        Some(if cancel { "cancelled" } else { "timed_out" })
    );
    assert_eq!(receipt.get("cleanup_confirmed"), Some(&Value::Bool(true)));
    if receipt
        .get("output_bytes")
        .and_then(Value::unsigned)
        .is_some_and(|bytes| bytes > 0)
    {
        assert_eq!(receipt.get("truncated"), Some(&Value::Bool(true)));
    }
}

#[test]
fn backpressured_start_notification_does_not_delay_cancellation() {
    backpressured_start(true);
}

#[test]
fn backpressured_start_notification_does_not_delay_timeout() {
    backpressured_start(false);
}
