use super::*;
use crate::{
    providers::openai_account::{Progress, Request, Response, client},
    tls::{Budget, NetworkError},
};
use std::{
    ops::ControlFlow,
    sync::{Mutex, atomic::AtomicUsize},
    time::{Duration, Instant},
};

#[derive(Default)]
struct Observed {
    logins: AtomicUsize,
    logouts: AtomicUsize,
    active: AtomicBool,
    cleaned: AtomicBool,
    requests: Mutex<Vec<String>>,
}
struct Backend {
    observed: Arc<Observed>,
    fail_first: bool,
    cancel_first: bool,
    active_work: bool,
}
impl worker::Backend for Backend {
    fn login(
        &mut self,
        budget: &Budget<'_>,
        code: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        let attempt = self.observed.logins.fetch_add(1, Ordering::AcqRel);
        if code("FAKE-ACCOUNT-CODE").is_break() {
            return Err(NetworkError::Cancelled.into());
        }
        if attempt == 0 && self.cancel_first {
            loop {
                budget.check()?;
                thread::sleep(Duration::from_millis(1));
            }
        }
        if attempt == 0 && self.fail_first {
            return Err(NetworkError::Timeout.into());
        }
        Ok(())
    }
    fn logout(&mut self, _: &Budget<'_>) -> Result<(), client::Error> {
        assert!(!self.observed.active.load(Ordering::Acquire));
        self.observed.logouts.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
    fn generate(
        &mut self,
        request: &Request,
        budget: &Budget<'_>,
        _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        self.observed
            .requests
            .lock()
            .unwrap()
            .push(request.encode(2 * 1024 * 1024)?);
        if self.active_work {
            self.observed.active.store(true, Ordering::Release);
            loop {
                if let Err(error) = budget.check() {
                    self.observed.active.store(false, Ordering::Release);
                    self.observed.cleaned.store(true, Ordering::Release);
                    return Err(error.into());
                }
                thread::sleep(Duration::from_millis(1));
            }
        }
        Ok(tests::response(
            "synthetic answer",
            crate::providers::openai_account::Status::Completed,
        ))
    }
}
fn backend(
    observed: &Arc<Observed>,
    fail_first: bool,
    cancel_first: bool,
    active_work: bool,
) -> Backend {
    Backend {
        observed: Arc::clone(observed),
        fail_first,
        cancel_first,
        active_work,
    }
}
fn next(run: &mut Session) -> Event {
    tests::next(run)
}

#[test]
fn failed_and_cancelled_sign_in_can_retry_without_recreating_the_conversation() {
    for cancelled_first in [false, true] {
        let observed = Arc::new(Observed::default());
        let mut run = Session::with_backend(
            Model::Luna,
            backend(&observed, !cancelled_first, cancelled_first, false),
            None,
        )
        .unwrap();
        assert!(matches!(next(&mut run), Event::LoginCode(_)));
        if cancelled_first {
            run.cancel();
        }
        assert!(matches!(next(&mut run), Event::LoginFailed(_)));
        assert!(run.signed_out());
        assert!(!run.submit("draft must stay local"));
        assert!(run.login());
        assert!(matches!(next(&mut run), Event::LoginCode(_)));
        assert!(matches!(next(&mut run), Event::Ready));
        assert!(
            !run.login(),
            "already signed in must not start another device flow"
        );
        assert_eq!(observed.logins.load(Ordering::Acquire), 2);
        assert!(run.logout());
        assert!(matches!(next(&mut run), Event::LoggedOut));
        assert!(run.signed_out());
        assert!(run.logout(), "repeated logout is idempotent");
        assert!(matches!(next(&mut run), Event::LoggedOut));
        assert_eq!(observed.logouts.load(Ordering::Acquire), 2);
        assert!(run.login());
        assert!(matches!(next(&mut run), Event::LoginCode(_)));
        assert!(matches!(next(&mut run), Event::Ready));
        assert!(
            observed.requests.lock().unwrap().is_empty(),
            "draft was never sent after login"
        );
    }
}

#[test]
fn active_logout_cancels_and_joins_work_before_signing_out() {
    let observed = Arc::new(Observed::default());
    let mut run =
        Session::with_backend(Model::Luna, backend(&observed, false, false, true), None).unwrap();
    assert!(matches!(next(&mut run), Event::LoginCode(_)));
    assert!(matches!(next(&mut run), Event::Ready));
    assert!(run.submit("a real prompt"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !observed.active.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(run.enqueue("queued guidance"));
    assert!(run.logout());
    assert!(run.signing_out());
    let mut finished = false;
    let mut returned = false;
    loop {
        match next(&mut run) {
            Event::Finished(End::Failed(Failure::Cancelled), _) => finished = true,
            Event::GuidanceReturned(text) if text == "queued guidance" => returned = true,
            Event::LoggedOut => break,
            Event::Text(_) | Event::Thinking | Event::RequestStarted => {}
            _ => {}
        }
    }
    assert!(finished && returned);
    assert!(observed.cleaned.load(Ordering::Acquire));
    assert_eq!(observed.logouts.load(Ordering::Acquire), 1);
    assert!(run.signed_out());
    assert!(run.login());
    assert!(matches!(next(&mut run), Event::LoginCode(_)));
    assert!(matches!(next(&mut run), Event::Ready));
    assert_eq!(
        observed.requests.lock().unwrap().len(),
        1,
        "no queued work resumed automatically"
    );
}

#[test]
fn logout_clears_guidance_at_an_idle_worker_boundary_before_relogin() {
    let observed = Arc::new(Observed::default());
    let mut run =
        Session::with_backend(Model::Luna, backend(&observed, false, false, false), None).unwrap();
    assert!(matches!(next(&mut run), Event::LoginCode(_)));
    assert!(matches!(next(&mut run), Event::Ready));
    assert!(run.submit("first request"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while observed.requests.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    // The UI has not consumed Finished yet and can still queue guidance.
    assert!(run.enqueue("unsent guidance"));
    assert!(run.logout());
    loop {
        if matches!(next(&mut run), Event::LoggedOut) {
            break;
        }
    }
    assert_eq!(run.queued, 0);
    assert!(run.login());
    loop {
        if matches!(next(&mut run), Event::Ready) {
            break;
        }
    }
    thread::sleep(Duration::from_millis(30));
    assert_eq!(observed.requests.lock().unwrap().len(), 1);
}

#[test]
fn obsolete_account_state_requires_a_real_login_before_another_prompt() {
    struct Stale(Arc<AtomicUsize>);
    impl worker::Backend for Stale {
        fn login(
            &mut self,
            _: &Budget<'_>,
            _: &mut dyn FnMut(&str) -> ControlFlow<()>,
        ) -> Result<(), client::Error> {
            self.0.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }
        fn generate(
            &mut self,
            _: &Request,
            _: &Budget<'_>,
            _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
        ) -> Result<Response, client::Error> {
            Err(client::Error::AccountChanged)
        }
    }
    let logins = Arc::new(AtomicUsize::new(0));
    let mut run = Session::with_backend(Model::Luna, Stale(Arc::clone(&logins)), None).unwrap();
    assert!(matches!(next(&mut run), Event::Ready));
    assert!(run.submit("once"));
    loop {
        if let Event::Finished(end, _) = next(&mut run) {
            assert_eq!(
                end,
                End::Failed(Failure::Account(client::Error::AccountChanged))
            );
            break;
        }
    }
    assert!(run.signed_out());
    assert!(!run.submit("must stay a draft"));
    assert!(run.login());
    assert!(matches!(next(&mut run), Event::Ready));
    assert_eq!(logins.load(Ordering::Acquire), 2);
}

#[test]
fn account_codes_and_transitions_never_enter_request_or_saved_history() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else {
        return;
    };
    let history = persistence::create_in(&store, Model::Luna, Some(&home.0), None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    let observed = Arc::new(Observed::default());
    let mut run = Session::with_history(
        Model::Luna,
        backend(&observed, false, false, false),
        None,
        history,
    )
    .unwrap();
    assert!(matches!(next(&mut run), Event::Restored { .. }));
    assert!(matches!(next(&mut run), Event::LoginCode(_)));
    assert!(matches!(next(&mut run), Event::Ready));
    assert!(run.logout());
    assert!(matches!(next(&mut run), Event::LoggedOut));
    assert!(run.login());
    assert!(matches!(next(&mut run), Event::LoginCode(_)));
    assert!(matches!(next(&mut run), Event::Ready));
    assert!(run.submit("actual request"));
    loop {
        if matches!(next(&mut run), Event::Finished(..)) {
            break;
        }
    }
    let requests = observed.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("actual request"));
    for forbidden in ["FAKE-ACCOUNT-CODE", "/login", "/logout"] {
        assert!(!requests[0].contains(forbidden));
    }
    drop(requests);
    drop(run);
    let saved = store
        .directory("sessions")
        .unwrap()
        .read(&format!("{id}.json"), 16 * 1024 * 1024)
        .unwrap()
        .unwrap();
    assert!(saved.contains("actual request"));
    for forbidden in ["FAKE-ACCOUNT-CODE", "/login", "/logout"] {
        assert!(!saved.contains(forbidden));
    }
}
