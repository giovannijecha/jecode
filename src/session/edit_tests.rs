//! Full controller tests with real isolated files and an inert model transport.
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
    let files = support::Fixture::new();
    files.write("notes.txt", "old\n");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let dropped = Arc::new(AtomicBool::new(false));
    let mut session = Session::with_backend(
        Model::Luna,
        Fixture {
            requests: requests.clone(),
            dropped: dropped.clone(),
        },
        Some(crate::workspace::Workspace::open(&files.0).unwrap()),
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
fn proposed(session: &mut Session) -> u64 {
    match tests::next(session) {
        Event::EditProposed { id, preview } => {
            assert_eq!(preview.path, "notes.txt");
            assert!(preview.diff.contains("- old\n+ new"));
            id
        }
        _ => panic!("expected complete edit proposal"),
    }
}
fn finish(session: &mut Session, allow: bool) -> (End, usize, Vec<String>) {
    let mut proposals = 0;
    let mut summaries = Vec::new();
    loop {
        match tests::next(session) {
            Event::EditProposed { id, .. } => {
                proposals += 1;
                assert!(session.decide(id, allow));
            }
            Event::EditFinished { summary, .. } => summaries.push(summary),
            Event::Finished(end, _) => return (end, proposals, summaries),
            Event::ToolStarted { .. }
            | Event::ToolFinished { .. }
            | Event::RequestStarted
            | Event::Text(_) => {}
            _ => panic!("unexpected event"),
        }
    }
}
#[test]
fn ordered_edits_wait_for_exact_decision_then_followup_reads_observe_the_result() {
    let mut run = start();
    assert!(!run.session.decide(1, true));
    assert!(run.session.submit("edit and inspect"));
    let id = proposed(&mut run.session);
    assert_eq!(
        fs::read_to_string(run.files.0.join("notes.txt")).unwrap(),
        "old\n"
    );
    assert_eq!(fs::read_dir(&run.files.0).unwrap().count(), 1);
    assert!(!run.session.decide(id + 1, true));
    assert!(!run.session.submit("overlap"));
    assert!(run.session.decide(id, true));
    assert!(!run.session.decide(id, true));
    let (end, proposals, summaries) = finish(&mut run.session, true);
    assert_eq!((end, proposals), (End::Complete, 1));
    let requests = run.requests.lock().unwrap();
    let results = outputs(&requests[1]);
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
        assert_eq!(
            results[1].1.get("text").and_then(Value::text),
            Some("old\n")
        );
        assert!(!run.files.0.join("created.txt").exists());
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
    drop(requests);
    assert!(run.session.submit("continue without repeating"));
    assert_eq!(finish(&mut run.session, true).1, 0);
    assert_eq!(outputs(&run.requests.lock().unwrap()[2]), results);
}
#[test]
fn denial_blocks_remaining_effects_in_the_turn_but_allows_read_results() {
    let mut run = start();
    assert!(run.session.submit("propose"));
    let id = proposed(&mut run.session);
    assert!(run.session.decide(id, false));
    let (end, later_proposals, _) = finish(&mut run.session, true);
    assert_eq!((end, later_proposals), (End::Complete, 0));
    assert_eq!(fs::read_dir(&run.files.0).unwrap().count(), 1);
    let requests = run.requests.lock().unwrap();
    let results = outputs(&requests[1]);
    assert_eq!(
        results[0].1.get("status").and_then(Value::text),
        Some("denied")
    );
    assert_eq!(
        results[1].1.get("text").and_then(Value::text),
        Some("old\n")
    );
    assert!(
        results[2]
            .1
            .get("error")
            .and_then(Value::text)
            .unwrap()
            .contains("after a denial")
    );
}
#[test]
fn changed_file_at_decision_time_is_preserved_and_failure_reaches_the_model() {
    let mut run = start();
    assert!(run.session.submit("propose"));
    let id = proposed(&mut run.session);
    run.files.write("notes.txt", "edited elsewhere\n");
    assert!(run.session.decide(id, true));
    finish(&mut run.session, false);
    assert_eq!(
        fs::read_to_string(run.files.0.join("notes.txt")).unwrap(),
        "edited elsewhere\n"
    );
    assert_eq!(fs::read_dir(&run.files.0).unwrap().count(), 1);
    let requests = run.requests.lock().unwrap();
    let results = outputs(&requests[1]);
    assert!(
        results[0]
            .1
            .get("error")
            .and_then(Value::text)
            .unwrap()
            .contains("changed since")
    );
}
#[test]
fn cancel_or_drop_while_approval_waits_has_no_effect_and_joins_worker() {
    for dropping in [false, true] {
        let mut run = start();
        assert!(run.session.submit("propose"));
        let id = proposed(&mut run.session);
        if !dropping {
            run.session.cancel();
            assert!(!run.session.decide(id, true));
            assert_eq!(
                finish(&mut run.session, true).0,
                End::Failed(Failure::Cancelled)
            );
            assert!(run.session.submit("new turn"));
            assert_eq!(finish(&mut run.session, true).1, 0);
            let requests = run.requests.lock().unwrap();
            assert!(
                outputs(&requests[1])[0]
                    .1
                    .get("error")
                    .and_then(Value::text)
                    .unwrap()
                    .contains("cancelled")
            );
        }
        drop(run.session);
        assert!(run.dropped.load(Ordering::Acquire));
        assert_eq!(fs::read_dir(&run.files.0).unwrap().count(), 1);
        assert_eq!(
            fs::read_to_string(run.files.0.join("notes.txt")).unwrap(),
            "old\n"
        );
    }
}
