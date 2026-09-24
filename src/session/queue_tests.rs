use crate::{
    providers::openai_account::{Progress, Request, Response, Status, client},
    session::{End, Event, Failure, Model, Session, tests, worker::Backend},
    tls::Budget,
};
use std::{
    ops::ControlFlow,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[test]
fn claim_and_withdraw_have_one_owner_and_keep_fifo() {
    use super::Pending;
    let pending = Arc::new(Pending::default());
    for text in ["first", "second", "third"] {
        assert!(pending.push(text));
    }
    assert_eq!(pending.withdraw_latest().as_deref(), Some("third"));
    assert_eq!(pending.claim().as_deref(), Some("first"));
    assert_eq!(pending.claim().as_deref(), Some("second"));
    assert_eq!(pending.claim(), None);

    // Both orders are forced with channels; no scheduler timing decides the result.
    assert!(pending.push("withdrawn"));
    let (go_tx, go_rx) = mpsc::sync_channel(0);
    let worker_queue = Arc::clone(&pending);
    let worker = std::thread::spawn(move || {
        go_rx.recv().unwrap();
        worker_queue.claim()
    });
    assert_eq!(pending.withdraw_latest().as_deref(), Some("withdrawn"));
    go_tx.send(()).unwrap();
    assert_eq!(worker.join().unwrap(), None);

    assert!(pending.push("claimed"));
    let (claimed_tx, claimed_rx) = mpsc::sync_channel(0);
    let worker_queue = Arc::clone(&pending);
    let worker = std::thread::spawn(move || {
        claimed_tx.send(worker_queue.claim()).unwrap();
    });
    assert_eq!(claimed_rx.recv().unwrap().as_deref(), Some("claimed"));
    assert_eq!(pending.withdraw_latest(), None);
    worker.join().unwrap();
}

struct Gated {
    requests: Arc<Mutex<Vec<String>>>,
    entered: mpsc::SyncSender<()>,
    release: mpsc::Receiver<()>,
}
impl Backend for Gated {
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
        let mut requests = self.requests.lock().unwrap();
        requests.push(request.encode(2 * 1024 * 1024)?);
        let first = requests.len() == 1;
        drop(requests);
        if first {
            self.entered.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        budget.check()?;
        Ok(tests::response("Completed step", Status::Completed))
    }
}

#[test]
fn latest_withdrawal_leaves_remaining_messages_in_delivery_order() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let mut run = Session::with_backend(
        Model::Luna,
        Gated {
            requests: Arc::clone(&requests),
            entered: entered_tx,
            release: release_rx,
        },
        None,
    )
    .unwrap();
    assert!(matches!(tests::next(&mut run), Event::Ready));
    assert!(run.submit("original task"));
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    for text in ["first guidance", "second guidance", "latest guidance"] {
        assert!(run.enqueue(text));
    }
    assert_eq!(run.withdraw_latest().as_deref(), Some("latest guidance"));
    assert_eq!(
        run.pending_guidance().snapshot(),
        ["first guidance", "second guidance"]
    );
    release_tx.send(()).unwrap();
    let mut accepted = Vec::new();
    loop {
        match tests::next(&mut run) {
            Event::Guidance { text, new_turn } => {
                assert!(!new_turn);
                accepted.push(text);
            }
            Event::Finished(End::Complete, _) => break,
            Event::Text(_) | Event::RequestStarted => {}
            _ => panic!("unexpected queued event"),
        }
    }
    assert_eq!(accepted, ["first guidance", "second guidance"]);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(!requests[0].contains("guidance"));
    let second = &requests[1];
    assert!(second.find("first guidance").unwrap() < second.find("second guidance").unwrap());
    assert!(!second.contains("latest guidance"));
}
struct Queued {
    requests: Arc<Mutex<Vec<String>>>,
    entered: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
}
impl Backend for Queued {
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
        self.requests
            .lock()
            .unwrap()
            .push(request.encode(2 * 1024 * 1024)?);
        self.entered.store(true, Ordering::Release);
        while !self.release.load(Ordering::Acquire) {
            budget.check()?;
            std::thread::sleep(Duration::from_millis(1));
        }
        budget.check()?;
        Ok(tests::response("Completed step", Status::Completed))
    }
}
#[test]
fn guidance_waits_for_a_model_boundary_and_cancel_returns_unsent_messages() {
    for cancel in [false, true] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let entered = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let mut run = Session::with_backend(
            Model::Luna,
            Queued {
                requests: requests.clone(),
                entered: entered.clone(),
                release: release.clone(),
            },
            None,
        )
        .unwrap();
        assert!(matches!(tests::next(&mut run), Event::Ready));
        assert!(run.submit("original task"));
        let deadline = Instant::now() + Duration::from_secs(2);
        while !entered.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(run.enqueue("use the second option"));
        assert_eq!(requests.lock().unwrap().len(), 1);
        if cancel {
            run.cancel();
        } else {
            release.store(true, Ordering::Release);
        }
        let mut accepted = false;
        let mut returned = false;
        loop {
            match tests::next(&mut run) {
                Event::Guidance { text, new_turn } => {
                    assert_eq!(text, "use the second option");
                    assert!(!new_turn);
                    accepted = true;
                }
                Event::GuidanceReturned(text) => {
                    assert_eq!(text, "use the second option");
                    returned = true;
                }
                Event::Finished(end, metrics) => {
                    assert_eq!(
                        end,
                        if cancel {
                            End::Failed(Failure::Cancelled)
                        } else {
                            End::Complete
                        }
                    );
                    assert_eq!(metrics.requests, if cancel { 1 } else { 2 });
                    break;
                }
                Event::Text(_) | Event::RequestStarted => {}
                _ => panic!("unexpected queued event"),
            }
        }
        assert_eq!(accepted, !cancel);
        assert_eq!(returned, cancel);
        if !cancel {
            let requests = requests.lock().unwrap();
            assert!(!requests[0].contains("second option"));
            assert!(
                requests[1].find("Completed step").unwrap()
                    < requests[1].find("second option").unwrap()
            );
        }
        drop(run);
    }
}
#[test]
fn failed_guidance_checkpoint_returns_claim_without_persisting_it() {
    let mut history = crate::session::history::History::default();
    history.begin("active objective".into()).unwrap();
    history.fail_next_checkpoint.store(true, Ordering::Release);
    let pending = Arc::new(super::Pending::default());
    assert!(pending.push("keep this guidance"));
    let (events, received) = mpsc::sync_channel(8);
    let context = crate::session::worker::Context {
        events,
        cancelled: Arc::new(AtomicBool::new(false)),
        stopped: Arc::new(AtomicBool::new(false)),
        guidance: pending.clone(),
        next_effect: std::sync::atomic::AtomicU64::new(1),
        effect_gate: None,
    };
    assert_eq!(super::take(&mut history, &context), Err(Failure::Storage));
    assert!(
        matches!(received.try_recv(), Ok(Event::GuidanceReturned(text)) if text == "keep this guidance")
    );
    assert!(history.turns[0].guidance.is_empty());
    assert!(pending.snapshot().is_empty());
}
