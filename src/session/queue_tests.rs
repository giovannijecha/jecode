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
    },
    time::{Duration, Instant},
};
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
