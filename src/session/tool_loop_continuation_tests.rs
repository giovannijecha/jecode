use super::*;
use crate::{
    json::{self, Value},
    providers::openai_account::{Progress, Request, Response, client},
    session::{Session, persistence, tests},
    tls::{Budget as NetworkBudget, IoOperation, NetworkError},
};
use std::{
    collections::VecDeque,
    ops::ControlFlow,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

enum Reply {
    Response(Response),
    Failure,
}
struct Scripted {
    replies: VecDeque<Reply>,
    requests: Arc<Mutex<Vec<Value>>>,
    gate: Option<(mpsc::SyncSender<()>, mpsc::Receiver<()>)>,
    cancel_first: bool,
}
impl Backend for Scripted {
    fn login(
        &mut self,
        _: &NetworkBudget<'_>,
        _: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        Ok(())
    }
    fn generate(
        &mut self,
        request: &Request,
        budget: &NetworkBudget<'_>,
        _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        budget.check()?;
        let body = json::parse(&request.encode(MAX_REQUEST)?, Default::default()).unwrap();
        let first = self.requests.lock().unwrap().is_empty();
        self.requests.lock().unwrap().push(body);
        if first {
            if let Some((entered, release)) = self.gate.take() {
                entered.send(()).unwrap();
                release.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            if self.cancel_first {
                budget
                    .cancelled
                    .store(true, std::sync::atomic::Ordering::Release);
            }
        }
        match self
            .replies
            .pop_front()
            .expect("unexpected extra generation")
        {
            Reply::Response(response) => Ok(response),
            Reply::Failure => Err(NetworkError::io(
                IoOperation::ReadRecordHeader,
                &std::io::Error::from(std::io::ErrorKind::ConnectionReset),
            )
            .into()),
        }
    }
}
fn response(text: &str, status: Status, end_turn: Option<bool>, phase: &str) -> Response {
    let mut response = tests::response(text, status);
    response.end_turn = end_turn;
    if let Value::Object(message) = &mut response.output[1] {
        message.insert("phase".into(), Value::String(phase.into()));
    }
    response
}
fn scripted(replies: Vec<Reply>) -> (Scripted, Arc<Mutex<Vec<Value>>>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    (
        Scripted {
            replies: replies.into(),
            requests: requests.clone(),
            gate: None,
            cancel_first: false,
        },
        requests,
    )
}
fn ready(session: &mut Session) {
    assert!(matches!(tests::next(session), Event::Ready));
}
fn finish(session: &mut Session) -> (End, Metrics) {
    loop {
        if let Event::Finished(end, metrics) = tests::next(session) {
            return (end, metrics);
        }
    }
}

#[test]
fn only_explicit_false_continues_a_completed_response_without_tools() {
    for signal in [None, Some(true), Some(false)] {
        let mut replies = vec![Reply::Response(response(
            "Progress.",
            Status::Completed,
            signal,
            "commentary",
        ))];
        if signal == Some(false) {
            replies.push(Reply::Response(response(
                "Done.",
                Status::Completed,
                Some(true),
                "final_answer",
            )));
        }
        let (backend, requests) = scripted(replies);
        let mut session = Session::with_backend(Model::Luna, backend, None).unwrap();
        ready(&mut session);
        assert!(session.submit("Do the task"));
        let (end, metrics) = finish(&mut session);
        assert_eq!(end, End::Complete);
        assert_eq!(metrics.requests, if signal == Some(false) { 2 } else { 1 });
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), metrics.requests as usize);
        if signal == Some(false) {
            let input = requests[1].get("input").and_then(Value::array).unwrap();
            assert_eq!(input.len(), 3);
            assert_eq!(
                input[0].get("content").and_then(Value::array).unwrap()[0]
                    .get("text")
                    .and_then(Value::text),
                Some("Do the task")
            );
            assert_eq!(
                input[1].get("type").and_then(Value::text),
                Some("reasoning")
            );
            assert_eq!(
                input[2].get("phase").and_then(Value::text),
                Some("commentary")
            );
            assert_eq!(
                input[2].get("content").and_then(Value::array).unwrap()[0]
                    .get("text")
                    .and_then(Value::text),
                Some("Progress.")
            );
        }
    }
}

#[test]
fn queued_guidance_joins_the_explicit_follow_up_once() {
    let (mut backend, requests) = scripted(vec![
        Reply::Response(response(
            "Progress.",
            Status::Completed,
            Some(false),
            "commentary",
        )),
        Reply::Response(response(
            "Done.",
            Status::Completed,
            Some(true),
            "final_answer",
        )),
    ]);
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    backend.gate = Some((entered_tx, release_rx));
    let mut session = Session::with_backend(Model::Luna, backend, None).unwrap();
    ready(&mut session);
    assert!(session.submit("Do the task"));
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(session.enqueue("Also check the edge case"));
    release_tx.send(()).unwrap();
    assert_eq!(finish(&mut session).0, End::Complete);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let input = json::encode(requests[1].get("input").unwrap(), MAX_REQUEST).unwrap();
    assert_eq!(input.matches("Also check the edge case").count(), 1);
}

#[test]
fn cancellation_and_terminal_results_do_not_trigger_extra_generation() {
    let (mut backend, requests) = scripted(vec![Reply::Response(response(
        "Progress.",
        Status::Completed,
        Some(false),
        "commentary",
    ))]);
    backend.cancel_first = true;
    let mut session = Session::with_backend(Model::Luna, backend, None).unwrap();
    ready(&mut session);
    assert!(session.submit("Do the task"));
    assert_eq!(finish(&mut session).0, End::Failed(Failure::Cancelled));
    assert_eq!(requests.lock().unwrap().len(), 1);

    for (status, expected) in [
        (Status::Incomplete, End::Incomplete),
        (Status::Refused, End::Refused),
    ] {
        let (backend, requests) = scripted(vec![Reply::Response(response(
            "Cannot proceed.",
            status,
            Some(false),
            "final_answer",
        ))]);
        let mut session = Session::with_backend(Model::Luna, backend, None).unwrap();
        ready(&mut session);
        assert!(session.submit("Do the task"));
        assert_eq!(finish(&mut session).0, expected);
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    let (backend, requests) = scripted(vec![
        Reply::Response(response(
            "Progress.",
            Status::Completed,
            Some(false),
            "commentary",
        )),
        Reply::Failure,
    ]);
    let mut session = Session::with_backend(Model::Luna, backend, None).unwrap();
    ready(&mut session);
    assert!(session.submit("Do the task"));
    assert!(matches!(
        finish(&mut session).0,
        End::Failed(Failure::Account(_))
    ));
    assert_eq!(requests.lock().unwrap().len(), 2);
}

#[test]
fn resumed_explicit_false_is_history_until_new_user_input() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let mut history = persistence::create_in(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("Earlier task".into()).unwrap();
    history.turns[0].steps.push(Step {
        text: "Earlier progress.".into(),
        response: Some(response(
            "Earlier progress.",
            Status::Completed,
            Some(false),
            "commentary",
        )),
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    history.checkpoint().unwrap();
    drop(history);

    let saved = persistence::load(&store, &id, true).unwrap();
    let (backend, requests) = scripted(vec![Reply::Response(response(
        "New answer.",
        Status::Completed,
        Some(true),
        "final_answer",
    ))]);
    let mut session = Session::with_history_shell(
        saved.model,
        backend,
        None,
        saved.history,
        crate::command::Shell::default(),
    )
    .unwrap();
    loop {
        match tests::next(&mut session) {
            Event::Restored { .. } => {}
            Event::Ready => break,
            _ => panic!("unexpected setup event"),
        }
    }
    assert!(requests.lock().unwrap().is_empty());
    assert!(session.submit("New request"));
    assert_eq!(finish(&mut session).0, End::Complete);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let input = json::encode(requests[0].get("input").unwrap(), MAX_REQUEST).unwrap();
    assert!(input.contains("Earlier progress."));
    assert!(input.contains("commentary"));
    assert!(input.contains("New request"));
}
