use super::*;
use crate::{
    providers::openai_account::{self as account, Input, Progress, Request, Response, Status},
    tls::{Budget, NetworkError},
};
use std::{
    ops::ControlFlow,
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Observed {
    requests: Mutex<Vec<String>>,
    dropped: AtomicBool,
    started: AtomicBool,
}
struct Fixture {
    observed: Arc<Observed>,
    fail_first: bool,
    flood: bool,
    catalog: Option<crate::providers::openai_account::catalog::Catalog>,
}
impl worker::Backend for Fixture {
    fn catalog(
        &mut self,
        _: &Budget<'_>,
    ) -> Result<Option<crate::providers::openai_account::catalog::Catalog>, client::Error> {
        Ok(self.catalog.clone())
    }
    fn login(
        &mut self,
        budget: &Budget<'_>,
        code: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        budget.check()?;
        if code("FAKE-DEVICE").is_break() {
            return Err(NetworkError::Cancelled.into());
        }
        Ok(())
    }
    fn generate(
        &mut self,
        request: &Request,
        budget: &Budget<'_>,
        progress: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        budget.check()?;
        self.observed
            .requests
            .lock()
            .unwrap()
            .push(request.encode(2 * 1024 * 1024)?);
        self.observed.started.store(true, Ordering::Release);
        if self.flood {
            loop {
                if progress(Progress::Text("x")).is_break() {
                    return Err(NetworkError::Cancelled.into());
                }
            }
        }
        if progress(Progress::Reasoning("brief reasoning summary")).is_break()
            || progress(Progress::Text("partial")).is_break()
        {
            return Err(NetworkError::Cancelled.into());
        }
        if self.fail_first && self.observed.requests.lock().unwrap().len() == 1 {
            return Err(NetworkError::io(
                crate::tls::IoOperation::ReadRecordHeader,
                &std::io::Error::from(std::io::ErrorKind::ConnectionReset),
            )
            .into());
        }
        Ok(response("partial completed", Status::Completed))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.observed.dropped.store(true, Ordering::Release);
    }
}
pub(super) fn response(text: &str, status: Status) -> Response {
    let output = crate::json::parse(&format!(r#"[{{"type":"reasoning","encrypted_content":"opaque-owned-fixture"}},{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"{text}"}}]}}]"#), Default::default()).unwrap();
    let crate::json::Value::Array(output) = output else {
        unreachable!()
    };
    Response {
        id: "fixture".into(),
        text: text.into(),
        output,
        status,
        tool_calls: vec![],
        usage: Default::default(),
    }
}
pub(crate) fn next(session: &mut Session) -> Event {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(event) = session.poll() {
            return event;
        }
        assert!(Instant::now() < deadline, "worker did not produce an event");
        thread::sleep(Duration::from_millis(1));
    }
}
fn start(fail_first: bool, flood: bool) -> (Session, Arc<Observed>) {
    let observed = Arc::new(Observed::default());
    let mut session = Session::with_backend(
        Model::Luna,
        Fixture {
            observed: Arc::clone(&observed),
            fail_first,
            flood,
            catalog: None,
        },
        None,
    )
    .unwrap();
    assert!(!session.submit("too early"));
    assert!(matches!(next(&mut session), Event::LoginCode(_)));
    assert!(matches!(next(&mut session), Event::Ready));
    (session, observed)
}
#[test]
fn unavailable_saved_pair_blocks_send_until_an_idle_replacement_is_applied() {
    let observed = Arc::new(Observed::default());
    let catalog = crate::providers::openai_account::catalog::Catalog::parse(br#"{"models":[{"slug":"replacement","visibility":"list","supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}]}]}"#).unwrap();
    let mut session = Session::with_backend(
        Model::Luna,
        Fixture {
            observed: Arc::clone(&observed),
            fail_first: false,
            flood: false,
            catalog: Some(catalog.clone()),
        },
        None,
    )
    .unwrap();
    assert!(matches!(next(&mut session), Event::LoginCode(_)));
    assert!(matches!(next(&mut session), Event::CatalogLoaded(_)));
    assert!(matches!(next(&mut session), Event::Ready));
    assert!(session.selection_unavailable());
    assert!(!session.submit("draft kept"));
    let unsupported = Model::new("replacement", Some("medium")).unwrap();
    assert!(!session.set_model(unsupported));
    let selected = Model::new("replacement", Some("high")).unwrap();
    assert!(session.set_model(selected));
    assert!(matches!(next(&mut session), Event::ModelChanged(value) if value == selected));
    assert!(session.submit("explicit request"));
    assert_eq!(finish(&mut session).1, End::Complete);
    assert_eq!(observed.requests.lock().unwrap().len(), 1);
}
pub(crate) fn ready_fixture() -> Session {
    start(false, false).0
}
pub(crate) fn ready_fixture_with_catalog(
    catalog: crate::providers::openai_account::catalog::Catalog,
) -> Session {
    let observed = Arc::new(Observed::default());
    let mut session = Session::with_backend(
        Model::Luna,
        Fixture {
            observed,
            fail_first: false,
            flood: false,
            catalog: Some(catalog),
        },
        None,
    )
    .unwrap();
    assert!(matches!(next(&mut session), Event::LoginCode(_)));
    assert!(matches!(next(&mut session), Event::CatalogLoaded(_)));
    assert!(matches!(next(&mut session), Event::Ready));
    session
}
fn finish(session: &mut Session) -> (String, End, Metrics) {
    let mut text = String::new();
    loop {
        match next(session) {
            Event::Text(delta) => text.push_str(&delta),
            Event::TextReconciled(whole) => text = whole,
            Event::Thinking => {}
            Event::Finished(end, metrics) => return (text, end, metrics),
            _ => panic!("unexpected event during generation"),
        }
    }
}

struct Boundaries;
impl worker::Backend for Boundaries {
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
        _: &Budget<'_>,
        progress: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        assert!(progress(Progress::Text("SameSame")).is_continue());
        let crate::json::Value::Array(output) = crate::json::parse(
            r#"[{"type":"message","id":"one","status":"completed","content":[{"type":"output_text","text":"Same"}]},{"type":"message","id":"two","status":"completed","content":[{"type":"output_text","text":"Same"}]}]"#,
            Default::default(),
        ).unwrap() else { unreachable!() };
        Ok(Response {
            id: "fixture".into(),
            status: Status::Completed,
            output,
            text: "Same\n\nSame".into(),
            tool_calls: Vec::new(),
            usage: Default::default(),
        })
    }
}

pub(crate) fn boundaries_fixture() -> Session {
    let mut session = Session::with_backend(Model::Luna, Boundaries, None).unwrap();
    assert!(matches!(next(&mut session), Event::Ready));
    session
}

#[test]
fn terminal_reconciliation_replaces_only_the_current_provisional_request() {
    let mut session = boundaries_fixture();
    assert!(session.submit("first"));
    assert!(matches!(next(&mut session), Event::Text(text) if text == "SameSame"));
    assert!(matches!(next(&mut session), Event::TextReconciled(text) if text == "Same\n\nSame"));
    assert!(matches!(
        next(&mut session),
        Event::Finished(End::Complete, _)
    ));
    assert!(session.submit("second"));
    assert_eq!(finish(&mut session).0, "Same\n\nSame");
}

#[test]
fn model_switch_is_ordered_and_next_request_keeps_the_conversation() {
    let (mut session, observed) = start(false, false);
    assert!(session.submit("first prompt"));
    assert!(!session.set_model(Model::Terra));
    assert_eq!(finish(&mut session).1, End::Complete);
    assert!(session.set_model(Model::Terra));
    assert!(!session.submit("too soon"));
    assert!(!session.enqueue("not guidance"));
    assert!(!session.set_model(Model::Luna));
    assert!(matches!(
        next(&mut session),
        Event::ModelChanged(Model::Terra)
    ));
    assert!(session.submit("continue with Terra"));
    assert_eq!(finish(&mut session).1, End::Complete);
    let requests = observed.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].contains("gpt-5.6-terra"));
    assert!(requests[1].contains("first prompt"));
    assert!(!requests[1].contains("too soon"));
}

#[test]
fn two_turns_preserve_order_opaque_output_and_final_only_suffix_without_duplication() {
    let (mut session, observed) = start(false, false);
    assert!(session.submit("first prompt"));
    assert!(!session.submit("overlapping prompt"));
    let (text, end, metrics) = finish(&mut session);
    assert_eq!(text, "partial completed");
    assert_eq!(end, End::Complete);
    assert!(metrics.first_text_ms.is_some());
    assert_eq!(metrics.input_tokens, None);
    assert_eq!(metrics.output_tokens, None);
    assert!(session.submit("second prompt"));
    assert_eq!(finish(&mut session).1, End::Complete);
    let requests = observed.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let second = &requests[1];
    assert!(second.find("first prompt").unwrap() < second.find("opaque-owned-fixture").unwrap());
    assert!(second.find("partial completed").unwrap() < second.find("second prompt").unwrap());
    assert!(!second.contains("FAKE-DEVICE"));
    assert!(!second.contains("overlapping prompt"));
    drop(requests);
    drop(session);
    assert!(observed.dropped.load(Ordering::Acquire));
}

#[test]
fn failure_is_not_retried_and_partial_text_is_not_fabricated_as_completed_context() {
    let (mut session, observed) = start(true, false);
    assert!(session.submit("first prompt"));
    let (text, end, _) = finish(&mut session);
    assert_eq!(text, "partial");
    assert!(matches!(end, End::Failed(Failure::Account(_))));
    assert_eq!(observed.requests.lock().unwrap().len(), 1);
    assert!(session.submit("explicit continuation"));
    assert_eq!(finish(&mut session).1, End::Complete);
    let requests = observed.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].contains("first prompt"));
    assert!(requests[1].contains("explicit continuation"));
    assert!(!requests[1].contains("partial"));
}

#[test]
fn dropping_backpressured_worker_cancels_and_joins_it() {
    let (mut session, observed) = start(false, true);
    assert!(session.submit("produce many tiny chunks"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !observed.started.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    // Deliberately do not consume the event channel. Joining must not wait for it.
    drop(session);
    assert!(observed.dropped.load(Ordering::Acquire));
}

#[test]
fn cancellation_leaves_partial_history_and_a_second_explicit_turn_can_start() {
    let (mut session, _) = start(false, true);
    assert!(session.submit("cancel me"));
    assert!(matches!(next(&mut session), Event::Text(_)));
    session.cancel();
    assert_eq!(finish(&mut session).1, End::Failed(Failure::Cancelled));
    assert!(session.submit("continue explicitly"));
    assert!(matches!(next(&mut session), Event::Text(_)));
    session.cancel();
    assert_eq!(finish(&mut session).1, End::Failed(Failure::Cancelled));
}

#[test]
fn incomplete_items_and_partial_attempts_stay_canonical_but_out_of_completed_projection() {
    let mut history = history::History::default();
    history.begin("first".into()).unwrap();
    history.turns[0].steps.push(history::Step {
        text: "not finished".into(),
        response: Some(response("not finished", Status::Incomplete)),
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Incomplete);
    history.begin("second".into()).unwrap();
    let request = history.request(Model::Terra, false).unwrap();
    assert_eq!(request.input.len(), 2);
    assert!(
        request
            .input
            .iter()
            .all(|input| matches!(input, Input::User(_)))
    );
    assert_eq!(history.turns[0].steps[0].text, "not finished");
    assert!(history.turns[0].steps[0].response.is_some());
    assert_eq!(request.model, "gpt-5.6-terra");
    assert!(request.tools.is_empty());
    assert_eq!(request.effort.as_deref(), Some("medium"));
    assert_eq!(account::auth::AUTH_HOST, "auth.openai.com");
}
