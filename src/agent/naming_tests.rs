use super::naming::clean;
use crate::{
    agent::{
        Agent,
        backend_tests::fixture::{Account, Reply},
    },
    attachments::{Inputs, Pool},
    backend::{Backend, Completion, Failure, Limits, Model, Service, Services, Update},
    cancel::Cancellation,
    effort::Effort,
    redact::Redactor,
    sessions::{Document, Handle},
    test_support::Directory,
    tools::Tools,
};
use std::{
    any::Any,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Default)]
struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

#[derive(Default)]
struct GateState {
    entered: bool,
    released: bool,
    finished: bool,
}

struct ReleaseOnDrop(Arc<Gate>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

impl Gate {
    fn wait_for(&self, done: impl Fn(&GateState) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut state = self.state.lock().unwrap();
        while !done(&state) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                drop(state);
                panic!("title worker did not reach expected state");
            }
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }

    fn release(&self) {
        self.state.lock().unwrap().released = true;
        self.changed.notify_all();
    }
}

#[derive(Clone)]
struct BlockingService {
    account: Account,
    gate: Arc<Gate>,
}

impl Service for BlockingService {
    fn id(&self) -> &'static str {
        self.account.id
    }
    fn with_credential(&self, _: String) -> Result<Arc<dyn Service>, String> {
        Ok(Arc::new(self.clone()))
    }
    fn with_cancellation(&self, _: Cancellation) -> Arc<dyn Service> {
        Arc::new(self.clone())
    }
    fn catalog(&self) -> Result<Vec<Model>, String> {
        Ok(vec![])
    }
    fn models(&self, _: &str) -> Result<Vec<Model>, String> {
        Ok(vec![])
    }
    fn check_credential(&self) -> Result<(), String> {
        Ok(())
    }
    fn redactor(&self) -> Redactor {
        Redactor::empty()
    }
    fn create(&self, model: String) -> Result<Box<dyn Backend>, String> {
        Ok(Box::new(BlockingBackend {
            inner: self.account.services().create(model)?,
            service: self.clone(),
            title_sampled: AtomicBool::new(false),
        }))
    }
}

struct BlockingBackend {
    inner: Box<dyn Backend>,
    service: BlockingService,
    title_sampled: AtomicBool,
}

impl Drop for BlockingBackend {
    fn drop(&mut self) {
        if self.title_sampled.load(Ordering::Acquire) {
            self.service.gate.state.lock().unwrap().finished = true;
            self.service.gate.changed.notify_all();
        }
    }
}

impl Backend for BlockingBackend {
    fn id(&self) -> &'static str {
        self.inner.id()
    }
    fn model(&self) -> &str {
        self.inner.model()
    }
    fn effort(&self) -> Effort {
        self.inner.effort()
    }
    fn set_effort(&mut self, effort: Effort) {
        self.inner.set_effort(effort);
    }
    fn inputs(&self) -> Inputs {
        self.inner.inputs()
    }
    fn limits(&mut self) -> Result<Option<Limits>, Failure> {
        self.inner.limits()
    }
    fn known_limits(&self) -> Option<Limits> {
        self.inner.known_limits()
    }
    fn sample(
        &self,
        messages: &[crate::json::Value],
        with_tools: bool,
        output_tokens: Option<usize>,
        pool: Option<&Pool>,
        emit: &mut dyn FnMut(Update) -> Result<(), String>,
    ) -> Result<Completion, Failure> {
        if !with_tools {
            self.title_sampled.store(true, Ordering::Release);
            let gate = &self.service.gate;
            let mut state = gate.state.lock().unwrap();
            state.entered = true;
            gate.changed.notify_all();
            while !state.released {
                state = gate.changed.wait(state).unwrap();
            }
        }
        self.inner
            .sample(messages, with_tools, output_tokens, pool, emit)
    }
    fn redactor(&self) -> Redactor {
        self.inner.redactor()
    }
    fn set_cancellation(&mut self, cancellation: Cancellation) {
        self.inner.set_cancellation(cancellation);
    }
    fn set_conversation(&mut self, id: Option<String>) {
        self.inner.set_conversation(id);
    }
    fn services(&self) -> Services {
        Services::from_service(self.service.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[test]
fn a_reply_becomes_one_clean_line() {
    assert_eq!(
        clean("\"Fix session resume.\"\nIt covers the parser.").as_deref(),
        Some("Fix session resume")
    );
    assert_eq!(
        clean("\n  # **Riprogettare la dashboard**  ").as_deref(),
        Some("Riprogettare la dashboard")
    );
    assert_eq!(clean("“”"), None);
    assert_eq!(clean(" \n "), None);
}

fn titled(handle: &Handle) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !handle.titled() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    handle.titled()
}

#[test]
fn the_first_request_names_the_session_once_and_the_title_is_saved() {
    let directory = Directory::new();
    let home = Directory::new();
    let account = Account::new("alpha", vec![Reply::text("Search index repair.")]);
    let mut agent = Agent::new(account.client(), Tools::new(directory.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    agent.name_sessions();
    agent
        .prepare_turn("Fix the empty-query path in session search")
        .unwrap();
    let handle = agent.sessions().unwrap();
    assert!(titled(&handle));
    let document = handle.snapshot();
    assert_eq!(document.title(), "Search index repair");
    let saved = Document::parse(&document.value()).unwrap();
    assert_eq!(saved.title.as_deref(), Some("Search index repair"));
    let request = account.requests.lock().unwrap()[0].encode();
    assert!(request.contains("Fix the empty-query path"), "{request}");

    handle.name("Another name").unwrap();
    assert_eq!(handle.snapshot().title(), "Search index repair");
}

#[test]
fn without_naming_the_title_stays_the_request() {
    let directory = Directory::new();
    let home = Directory::new();
    let account = Account::new("alpha", vec![]);
    let mut agent = Agent::new(account.client(), Tools::new(directory.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    agent.prepare_turn("Fix the empty-query path").unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let handle = agent.sessions().unwrap();
    assert!(!handle.titled());
    assert_eq!(handle.snapshot().title(), "Fix the empty-query path");
    assert!(account.requests.lock().unwrap().is_empty());
}

#[test]
fn an_in_flight_title_does_not_keep_a_switched_session_open_or_recreate_it() {
    let directory = Directory::new();
    let home = Directory::new();
    let account = Account::new(
        "alpha",
        vec![
            Reply::text("Main answer"),
            Reply::text("Late generated title"),
        ],
    );
    let gate = Arc::new(Gate::default());
    let _release_on_drop = ReleaseOnDrop(gate.clone());
    let service = BlockingService {
        account,
        gate: gate.clone(),
    };
    let client = service.create("shared-model-name".into()).unwrap();
    let mut agent = Agent::with_backend(client, Tools::new(directory.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    agent.name_sessions();
    agent.run_turn("First request", &mut |_| Ok(())).unwrap();
    gate.wait_for(|state| state.entered);
    let old = agent.sessions().unwrap();
    let store = old.store();
    let id = old.id();
    drop(old);

    agent
        .start_new("shared-model-name".into(), Effort::Default)
        .unwrap();
    let (reopened, _) = Handle::open(store.clone(), &id, Redactor::empty()).unwrap();
    assert!(!reopened.titled());
    drop(reopened);
    store.delete(&id).unwrap();
    gate.release();
    gate.wait_for(|state| state.finished);
    assert!(store.read(&id).is_err());
}
