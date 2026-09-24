//! Full controller tests with isolated files and an inert model transport.
use super::tool_tests::{call, calls_response, outputs};
use super::*;
use crate::workspace_fixture as support;
use crate::{
    json::Value,
    providers::openai_account::{Progress, Request, Response, Status},
    tls::Budget,
};
use std::{fs, ops::ControlFlow, sync::Mutex};

struct Fixture {
    requests: Arc<Mutex<Vec<String>>>,
    dropped: Arc<AtomicBool>,
}
impl worker::Backend for Fixture {
    fn login(
        &mut self,
        _: &Budget<'_>,
        _: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        Ok(())
    }
    fn generate(
        &mut self,
        request: &Request,
        _: &Budget<'_>,
        _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        let mut requests = self.requests.lock().unwrap();
        requests.push(request.encode(2 * 1024 * 1024)?);
        Ok(if requests.len() == 1 {
            calls_response(vec![
                call(
                    "edit",
                    "edit_file",
                    r#"{"path":"notes.txt","old_text":"old","new_text":"new"}"#,
                ),
                call("read", "read_file", r#"{"path":"notes.txt"}"#),
                call(
                    "create",
                    "create_file",
                    r#"{"path":"created.txt","content":"created\n"}"#,
                ),
            ])
        } else {
            tests::response("Recorded the tool outcomes.", Status::Completed)
        })
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::Release);
    }
}
pub(crate) struct Run {
    pub session: Session,
    pub files: support::Fixture,
    requests: Arc<Mutex<Vec<String>>>,
    dropped: Arc<AtomicBool>,
}
pub(crate) fn start() -> Run {
    start_with_gate(None)
}
fn start_with_gate(gate: Option<worker::EffectGate>) -> Run {
    let files = support::Fixture::new();
    files.write("notes.txt", "old\n");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let dropped = Arc::new(AtomicBool::new(false));
    let history = history::History {
        effect_gate: gate,
        ..Default::default()
    };
    let mut session = Session::with_history(
        Model::Luna,
        Fixture {
            requests: requests.clone(),
            dropped: dropped.clone(),
        },
        Some(crate::workspace::Workspace::open(&files.0).unwrap()),
        history,
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    Run {
        session,
        files,
        requests,
        dropped,
    }
}
pub(crate) fn next(session: &mut Session) -> Event {
    tests::next(session)
}
fn finish(session: &mut Session) -> (End, Vec<String>, usize, usize) {
    let mut plans = 0;
    let mut summaries = Vec::new();
    loop {
        match next(session) {
            Event::EditPlanned { .. } => plans += 1,
            Event::EditFinished { summary, .. } => summaries.push(summary),
            Event::Finished(end, metrics) => {
                return (end, summaries, metrics.tool_calls as usize, plans);
            }
            Event::ToolStarted { .. }
            | Event::ToolFinished { .. }
            | Event::RequestStarted
            | Event::Text(_) => {}
            _ => panic!("unexpected event"),
        }
    }
}

#[test]
fn completed_edit_receipt_does_not_wait_for_a_full_presentation_queue() {
    use std::{sync::mpsc, time::Duration};

    let files = support::Fixture::new();
    files.write("notes.txt", "old\n");
    let workspace = crate::workspace::Workspace::open(&files.0).unwrap();
    let args = crate::json::parse(
        r#"{"path":"notes.txt","old_text":"old","new_text":"new"}"#,
        Default::default(),
    )
    .unwrap();
    let tool = crate::tools::Prepared::parse("edit_file", &args).unwrap();
    let (events, received) = mpsc::sync_channel(64);
    for _ in 0..63 {
        events.send(Event::Thinking).unwrap();
    }
    let stopped = Arc::new(AtomicBool::new(false));
    let context = worker::Context {
        events,
        cancelled: Arc::new(AtomicBool::new(false)),
        stopped: stopped.clone(),
        guidance: Arc::new(queue::Pending::default()),
        next_effect: std::sync::atomic::AtomicU64::new(1),
        effect_gate: None,
    };
    let (done, completed) = mpsc::sync_channel(1);
    let handle = std::thread::spawn(move || {
        let _ = done.send(super::edit::execute(tool, &workspace, &context).0);
    });
    let before_release = completed.recv_timeout(Duration::from_secs(3)).ok();
    let receipt_ready_before_release = before_release.is_some();
    stopped.store(true, Ordering::Release);
    drop(received);
    let outcome = before_release.or_else(|| completed.recv_timeout(Duration::from_secs(10)).ok());
    handle.join().unwrap();
    let output = outcome.expect("edit did not finish after presentation was released");
    assert!(receipt_ready_before_release);
    assert_eq!(
        fs::read_to_string(files.0.join("notes.txt")).unwrap(),
        "new\n"
    );
    let receipt = crate::json::parse(&output.text, Default::default()).unwrap();
    assert_eq!(receipt.get("status").and_then(Value::text), Some("applied"));
}

#[test]
fn direct_edits_and_create_follow_order_and_reads_observe_the_result() {
    let mut run = start();
    assert!(run.session.submit("edit and inspect"));
    let (end, summaries, calls, plans) = finish(&mut run.session);
    assert_eq!((end, calls, plans), (End::Complete, 3, 2));
    assert_eq!(summaries.len(), 2);
    let results = outputs(&run.requests.lock().unwrap()[1]);
    assert_eq!(
        results
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["edit", "read", "create"]
    );
    if summaries
        .iter()
        .any(|s| run.files.unsupported_host_filesystem(s))
    {
        assert_eq!(results[0].1.get("ok"), Some(&Value::Bool(false)));
    } else {
        assert_eq!(
            results[0].1.get("status").and_then(Value::text),
            Some("applied")
        );
        assert_eq!(
            results[1].1.get("text").and_then(Value::text),
            Some("new\n")
        );
        assert_eq!(
            fs::read_to_string(run.files.0.join("created.txt")).unwrap(),
            "created\n"
        );
        let recovery = results[0].1.get("recovery").and_then(Value::text).unwrap();
        assert_eq!(
            fs::read_to_string(run.files.0.join(recovery)).unwrap(),
            "old\n"
        );
    }
    assert!(run.session.submit("continue without repeating"));
    assert_eq!(finish(&mut run.session).2, 0);
    assert_eq!(outputs(&run.requests.lock().unwrap()[2]), results);
}

#[test]
fn competing_edit_between_prepare_and_apply_is_preserved() {
    let files = Arc::new(Mutex::new(None::<std::path::PathBuf>));
    let target = files.clone();
    let gate: worker::EffectGate = Arc::new(move |name| {
        if name == "edit_file" {
            fs::write(
                target.lock().unwrap().as_ref().unwrap().join("notes.txt"),
                "edited elsewhere\n",
            )
            .unwrap();
        }
    });
    let mut run = start_with_gate(Some(gate));
    *files.lock().unwrap() = Some(run.files.0.clone());
    assert!(run.session.submit("edit"));
    assert_eq!(finish(&mut run.session).0, End::Complete);
    assert_eq!(
        fs::read_to_string(run.files.0.join("notes.txt")).unwrap(),
        "edited elsewhere\n"
    );
    let results = outputs(&run.requests.lock().unwrap()[1]);
    assert_eq!(
        results[0].1.get("status").and_then(Value::text),
        Some("failed")
    );
    assert!(
        results[0]
            .1
            .get("error")
            .and_then(Value::text)
            .unwrap()
            .contains("changed since")
    );
    assert_eq!(
        results[1].1.get("text").and_then(Value::text),
        Some("edited elsewhere\n")
    );
}

#[test]
fn cancellation_before_execution_prevents_change_and_joins_worker() {
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let release = Arc::new(Mutex::new(release_rx));
    let gate: worker::EffectGate = Arc::new(move |name| {
        if name == "edit_file" {
            entered_tx.send(()).unwrap();
            release.lock().unwrap().recv().unwrap();
        }
    });
    let mut run = start_with_gate(Some(gate));
    assert!(run.session.submit("edit"));
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    run.session.cancel();
    release_tx.send(()).unwrap();
    assert_eq!(finish(&mut run.session).0, End::Failed(Failure::Cancelled));
    assert_eq!(
        fs::read_to_string(run.files.0.join("notes.txt")).unwrap(),
        "old\n"
    );
    assert!(!run.files.0.join("created.txt").exists());
    drop(run.session);
    assert!(run.dropped.load(Ordering::Acquire));
}

#[test]
fn forty_direct_effects_keep_exact_order_and_run_once() {
    struct EffectChain {
        round: usize,
        requests: Arc<Mutex<Vec<String>>>,
    }
    impl worker::Backend for EffectChain {
        fn login(
            &mut self,
            _: &Budget<'_>,
            _: &mut dyn FnMut(&str) -> ControlFlow<()>,
        ) -> Result<(), client::Error> {
            Ok(())
        }
        fn generate(
            &mut self,
            request: &Request,
            budget: &Budget<'_>,
            _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
        ) -> Result<Response, client::Error> {
            budget.check()?;
            let encoded = request.encode(2 * 1024 * 1024)?;
            if self.round > 0 {
                let receipts = outputs(&encoded);
                assert_eq!(receipts.len(), self.round);
                assert_eq!(
                    receipts.last().unwrap().0,
                    format!("edit-{}", self.round - 1)
                );
                assert_eq!(
                    receipts
                        .last()
                        .unwrap()
                        .1
                        .get("status")
                        .and_then(Value::text),
                    Some("applied")
                );
            }
            self.requests.lock().unwrap().push(encoded);
            if self.round == 40 {
                return Ok(tests::response(
                    "Verified all ordered edits.",
                    Status::Completed,
                ));
            }
            let old = (0..self.round).fold("start".to_owned(), |mut text, n| {
                text.push_str(&format!(" {n}"));
                text
            });
            let new = format!("{old} {}", self.round);
            let id = format!("edit-{}", self.round);
            self.round += 1;
            Ok(calls_response(vec![call(
                &id,
                "edit_file",
                &format!(r#"{{"path":"notes.txt","old_text":"{old}","new_text":"{new}"}}"#),
            )]))
        }
    }
    let files = support::Fixture::new();
    files.write("notes.txt", "start");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_backend(
        Model::Luna,
        EffectChain {
            round: 0,
            requests: requests.clone(),
        },
        Some(crate::workspace::Workspace::open(&files.0).unwrap()),
    )
    .unwrap();
    assert!(matches!(next(&mut session), Event::Ready));
    assert!(session.submit("apply all forty"));
    assert_eq!(finish(&mut session).0, End::Complete);
    assert_eq!(requests.lock().unwrap().len(), 41);
    assert!(
        fs::read_to_string(files.0.join("notes.txt"))
            .unwrap()
            .ends_with(" 39")
    );
}
