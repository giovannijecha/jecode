//! Exercise the real 64-event presentation queue with preloaded model bytes.
use super::*;
use crate::{
    providers::openai_account::{Progress, Request, Response, client},
    tls::Budget,
};
use std::{
    ops::ControlFlow,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};

struct ReadyBackend {
    chunks: Vec<Vec<u8>>,
    clock: Arc<Mutex<Instant>>,
    entered: Sender<()>,
    returned: Sender<bool>,
    reads: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
}
impl worker::Backend for ReadyBackend {
    fn login(
        &mut self,
        _: &Budget<'_>,
        _: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        Ok(())
    }
    fn generate(
        &mut self,
        _: &Request,
        budget: &Budget<'_>,
        progress: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        self.requests.fetch_add(1, Ordering::Release);
        let mut text_events = 0;
        let result = client::timing_fixture::exchange_ready(
            std::mem::take(&mut self.chunks),
            budget,
            Duration::from_secs(120),
            Arc::clone(&self.reads),
            &mut |event| {
                if matches!(event, Progress::Text(_)) {
                    text_events += 1;
                    if text_events == 65 {
                        let _ = self.entered.send(());
                    }
                }
                progress(event)
            },
            || *self.clock.lock().unwrap(),
        );
        let _ = self.returned.send(result.is_ok());
        result
    }
}

struct Run {
    session: Session,
    entered: Receiver<()>,
    returned: Receiver<bool>,
    clock: Arc<Mutex<Instant>>,
    origin: Instant,
    reads: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
    expected: String,
}
fn start(history: history::History) -> Run {
    let origin = Instant::now();
    let clock = Arc::new(Mutex::new(origin));
    let reads = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(AtomicUsize::new(0));
    let (entered_tx, entered) = mpsc::channel();
    let (returned_tx, returned) = mpsc::channel();
    let mut first_body = String::new();
    let mut expected = String::new();
    for index in 0..65 {
        let part = format!("{index:02}|");
        expected.push_str(&part);
        first_body.push_str(&format!(
            "data: {{\"type\":\"response.output_text.delta\",\"delta\":\"{part}\"}}\n\n"
        ));
    }
    let terminal = format!(
        "data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"r1\",\"status\":\"completed\",\"output\":[{{\"type\":\"message\",\"content\":[{{\"type\":\"output_text\",\"text\":\"{expected}\"}}]}}]}}}}\n\n"
    );
    let first = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{first_body}",
        first_body.len() + terminal.len()
    );
    let backend = ReadyBackend {
        chunks: vec![first.into_bytes(), terminal.into_bytes()],
        clock: Arc::clone(&clock),
        entered: entered_tx,
        returned: returned_tx,
        reads: Arc::clone(&reads),
        requests: Arc::clone(&requests),
    };
    let mut session = Session::with_history(Model::Luna, backend, None, history).unwrap();
    if matches!(tests::next(&mut session), Event::Restored { .. }) {
        assert!(matches!(tests::next(&mut session), Event::Ready));
    }
    assert!(session.submit("verify bounded presentation"));
    Run {
        session,
        entered,
        returned,
        clock,
        origin,
        reads,
        requests,
        expected,
    }
}
fn blocked(run: &Run) {
    run.entered.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(run.reads.load(Ordering::Acquire), 1);
    assert!(matches!(
        run.returned.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    *run.clock.lock().unwrap() = run.origin + Duration::from_secs(200);
}
fn finish(session: &mut Session) -> (Vec<String>, End) {
    let mut text = Vec::new();
    loop {
        match tests::next(session) {
            Event::Text(piece) => text.push(piece),
            Event::Finished(end, _) => return (text, end),
            _ => {}
        }
    }
}

#[test]
fn delayed_drain_keeps_ordered_text_and_durable_completion() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let history = persistence::create_in(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    let mut run = start(history);
    blocked(&run);
    let (text, end) = finish(&mut run.session);
    assert_eq!(end, End::Complete);
    assert_eq!(text.concat(), run.expected);
    assert_eq!(text.len(), 65);
    assert_eq!(run.returned.recv_timeout(Duration::from_secs(1)), Ok(true));
    assert_eq!(run.reads.load(Ordering::Acquire), 2);
    assert_eq!(run.requests.load(Ordering::Acquire), 1);
    drop(run);
    let saved = persistence::load(&store, &id, false).unwrap();
    let turn = saved.history.turns.last().unwrap();
    assert_eq!(turn.end, Some(End::Complete));
    assert_eq!(turn.steps[0].response.as_ref().unwrap().text, text.concat());
}

#[test]
fn cancellation_interrupts_full_presentation_without_replaying_request() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else { return };
    let history = persistence::create_in(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    let mut run = start(history);
    blocked(&run);
    run.session.cancel();
    assert_eq!(run.returned.recv_timeout(Duration::from_secs(1)), Ok(false));
    let (text, end) = finish(&mut run.session);
    assert_eq!(end, End::Failed(Failure::Cancelled));
    assert_eq!(run.reads.load(Ordering::Acquire), 1);
    assert_eq!(run.requests.load(Ordering::Acquire), 1);
    let expected = run.expected.clone();
    drop(run);
    let saved = persistence::load(&store, &id, false).unwrap();
    let turn = saved.history.turns.last().unwrap();
    // Persisted failure kinds are intentionally normalized on load; the
    // retained outcome and partial text carry the interruption evidence.
    assert_eq!(turn.end, Some(End::Failed(Failure::Worker)));
    assert_eq!(turn.outcome, Failure::Cancelled.to_string());
    // The cancellation can win just before or just after the 65th delta is
    // accepted: `entered` is sent before passing that delta to the controller.
    let saved = &turn.steps[0].text;
    assert!([64 * 3, 65 * 3].contains(&saved.len()));
    assert!(expected.starts_with(saved));
    assert!(expected.starts_with(&text.concat()));
}

#[test]
fn shutdown_joins_worker_while_presentation_is_full() {
    let run = start(history::History::default());
    blocked(&run);
    let started = Instant::now();
    drop(run.session);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(run.returned.recv_timeout(Duration::from_secs(1)), Ok(false));
    assert_eq!(run.reads.load(Ordering::Acquire), 1);
    assert_eq!(run.requests.load(Ordering::Acquire), 1);
}
