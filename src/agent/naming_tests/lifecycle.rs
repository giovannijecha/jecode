use super::*;
use crate::{events::Event, json::Value};
use std::sync::mpsc;

struct Fixture {
    agent: Option<Agent>,
    gate: Arc<Gate>,
    _project: Directory,
    _home: Directory,
}

impl Fixture {
    fn new(outcome: TitleOutcome) -> Self {
        let project = Directory::new();
        let home = Directory::new();
        let gate = Arc::new(Gate::default());
        let service = BlockingService {
            account: Account::new(
                "alpha",
                vec![Reply::text("Main answer"), Reply::text("Generated title")],
            ),
            gate: gate.clone(),
            outcome,
        };
        let mut agent = Agent::new(service.client(false), Tools::new(project.path()).unwrap());
        agent.set_effort(Effort::High);
        agent.enable_sessions(home.path()).unwrap();
        agent.name_sessions();
        Self {
            agent: Some(agent),
            gate,
            _project: project,
            _home: home,
        }
    }
}

fn title_event(document: &Document) -> &Value {
    let events = document
        .events
        .iter()
        .filter(|event| event.get("operation").and_then(Value::as_str) == Some("session_title"))
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1, "{events:?}");
    events[0]
}

fn partial_usage() -> Usage {
    Usage {
        input_tokens: Some(17),
        cached_tokens: Some(0),
        ..Usage::default()
    }
}

struct Running {
    worker: Option<std::thread::JoinHandle<(Agent, Result<(), String>)>>,
    gate: Arc<Gate>,
}

impl Running {
    fn start(
        mut agent: Agent,
        gate: Arc<Gate>,
        mut sink: impl FnMut(Event) -> Result<(), String> + Send + 'static,
    ) -> Self {
        Self {
            worker: Some(std::thread::spawn(move || {
                let result = agent.run_single_turn("First request", &mut sink);
                (agent, result)
            })),
            gate,
        }
    }

    fn is_finished(&self) -> bool {
        self.worker.as_ref().unwrap().is_finished()
    }

    fn finish(mut self) -> (Agent, Result<(), String>) {
        self.worker.take().unwrap().join().unwrap()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.gate.release();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[test]
fn one_shot_streams_the_main_answer_then_waits_for_the_title_and_its_lease() {
    for usage in [None, Some(partial_usage())].into_iter().cycle().take(8) {
        let mut fixture = Fixture::new(TitleOutcome::Complete(usage));
        let agent = fixture.agent.take().unwrap();
        let handle = agent.sessions().unwrap();
        let (sent, received) = mpsc::channel();
        let worker = Running::start(agent, fixture.gate.clone(), move |event| {
            if matches!(event, Event::Message { .. }) {
                sent.send(()).unwrap();
            }
            Ok(())
        });
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        fixture.gate.wait_for(|state| state.entered);
        assert!(!worker.is_finished());
        assert!(handle.leased());
        assert!(Handle::open(handle.store(), &handle.id(), Redactor::empty()).is_err());
        fixture.gate.release();
        let (agent, result) = worker.finish();
        result.unwrap();
        let saved = handle.store().read(&handle.id()).unwrap();
        assert_eq!(saved.title.as_deref(), Some("Generated title"));
        let event = title_event(&saved);
        assert_eq!(
            event.get("status").and_then(Value::as_str),
            Some("complete")
        );
        assert_eq!(event.get("effort").and_then(Value::as_str), Some("low"));
        assert_eq!(
            event.get("input_tokens").and_then(Value::as_usize),
            usage.and_then(|usage| usage.input_tokens)
        );
        assert_eq!(
            event.get("cached_tokens").and_then(Value::as_usize),
            usage.and_then(|usage| usage.cached_tokens)
        );
        for field in [
            "output_tokens",
            "total_tokens",
            "cache_write_tokens",
            "reasoning_tokens",
        ] {
            assert_eq!(event.get(field), Some(&Value::Null), "{field}");
        }
        assert_eq!(agent.effort(), Effort::High);
        let store = handle.store();
        let id = handle.id();
        drop(agent);
        drop(handle);
        let (reopened, _) = Handle::open(store, &id, Redactor::empty()).unwrap();
        assert_eq!(title_event(&reopened.snapshot()), event);
    }
}

#[test]
fn provider_and_preparation_failures_save_one_attempt_without_failing_the_main_answer() {
    for outcome in [
        TitleOutcome::Failed(None),
        TitleOutcome::Failed(Some(partial_usage())),
        TitleOutcome::PrepareFailure,
        TitleOutcome::CreateFailure,
    ] {
        let mut fixture = Fixture::new(outcome);
        fixture.gate.release();
        let mut agent = fixture.agent.take().unwrap();
        agent
            .run_single_turn("First request", &mut |_| Ok(()))
            .unwrap();
        let handle = agent.sessions().unwrap();
        let saved = handle.store().read(&handle.id()).unwrap();
        assert!(saved.title.is_none());
        assert_eq!(saved.title(), "First request");
        let event = title_event(&saved);
        assert_eq!(event.get("status").and_then(Value::as_str), Some("failed"));
        assert!(
            event
                .get("error")
                .and_then(Value::as_str)
                .unwrap()
                .contains("fixture title")
        );
        assert_eq!(
            event.get("input_tokens").and_then(Value::as_usize),
            if matches!(outcome, TitleOutcome::Failed(Some(_))) {
                Some(17)
            } else {
                None
            }
        );
        assert_eq!(event.get("output_tokens"), Some(&Value::Null));
    }
}

#[test]
fn a_main_output_error_still_waits_for_and_saves_the_title() {
    let mut fixture = Fixture::new(TitleOutcome::Complete(None));
    let agent = fixture.agent.take().unwrap();
    let handle = agent.sessions().unwrap();
    let (sent, received) = mpsc::channel();
    let worker = Running::start(agent, fixture.gate.clone(), move |event| {
        if matches!(event, Event::Message { .. }) {
            sent.send(()).unwrap();
            return Err("fixture output failed".into());
        }
        Ok(())
    });
    received.recv_timeout(Duration::from_secs(5)).unwrap();
    fixture.gate.wait_for(|state| state.entered);
    assert!(!worker.is_finished());
    fixture.gate.release();
    let (_agent, result) = worker.finish();
    assert!(result.unwrap_err().contains("fixture output failed"));
    let saved = handle.store().read(&handle.id()).unwrap();
    assert_eq!(
        title_event(&saved).get("status").and_then(Value::as_str),
        Some("complete")
    );
}

#[test]
fn cancellation_saves_streamed_usage_before_the_turn_token_can_be_reset() {
    let mut fixture = Fixture::new(TitleOutcome::Failed(Some(partial_usage())));
    let agent = fixture.agent.as_mut().unwrap();
    agent.prepare_turn("First request").unwrap();
    fixture.gate.wait_for(|state| state.entered);
    agent.cancellation().cancel();
    let error = agent
        .run_single_turn("First request", &mut |_| Ok(()))
        .unwrap_err();
    assert!(error.contains("cancelled"));
    assert!(fixture.gate.state.lock().unwrap().finished);
    agent.cancellation().reset();
    let handle = agent.sessions().unwrap();
    let saved = handle.store().read(&handle.id()).unwrap();
    let event = title_event(&saved);
    assert_eq!(
        event.get("status").and_then(Value::as_str),
        Some("cancelled")
    );
    assert_eq!(
        event.get("input_tokens").and_then(Value::as_usize),
        Some(17)
    );
    assert_eq!(event.get("output_tokens"), Some(&Value::Null));
}

#[test]
fn dropping_an_interactive_agent_joins_cancellation_before_releasing_the_session() {
    let mut fixture = Fixture::new(TitleOutcome::Failed(None));
    let mut agent = fixture.agent.take().unwrap();
    agent.run_turn("First request", &mut |_| Ok(())).unwrap();
    fixture.gate.wait_for(|state| state.entered);
    let handle = agent.sessions().unwrap();
    let store = handle.store();
    let id = handle.id();
    drop(handle);
    drop(agent);
    assert!(fixture.gate.state.lock().unwrap().finished);
    let (reopened, _) = Handle::open(store, &id, Redactor::empty()).unwrap();
    assert_eq!(
        title_event(&reopened.snapshot())
            .get("status")
            .and_then(Value::as_str),
        Some("cancelled")
    );
}

#[test]
fn deleting_the_current_session_joins_the_title_before_removing_its_files() {
    let mut fixture = Fixture::new(TitleOutcome::Failed(None));
    let agent = fixture.agent.as_mut().unwrap();
    agent.run_turn("First request", &mut |_| Ok(())).unwrap();
    fixture.gate.wait_for(|state| state.entered);
    let handle = agent.sessions().unwrap();
    let store = handle.store();
    let id = handle.id();
    drop(handle);
    agent
        .delete_session(&id, agent.model().into(), Effort::High)
        .unwrap();
    assert!(fixture.gate.state.lock().unwrap().finished);
    assert!(store.read(&id).is_err());
}

#[test]
fn resuming_another_session_joins_the_old_title_without_changing_the_new_history() {
    let mut fixture = Fixture::new(TitleOutcome::Failed(None));
    let mut alternate = Agent::new(
        Account::new("alpha", vec![]).client(),
        Tools::new(fixture._project.path()).unwrap(),
    );
    alternate.enable_sessions(fixture._home.path()).unwrap();
    alternate.prepare_turn("Other request").unwrap();
    let next_id = alternate.sessions().unwrap().id();
    drop(alternate);

    let agent = fixture.agent.as_mut().unwrap();
    agent.run_turn("First request", &mut |_| Ok(())).unwrap();
    fixture.gate.wait_for(|state| state.entered);
    let old = agent.sessions().unwrap();
    let store = old.store();
    let id = old.id();
    drop(old);
    agent.resume(&next_id).unwrap();
    assert!(fixture.gate.state.lock().unwrap().finished);
    let (reopened, _) = Handle::open(store, &id, Redactor::empty()).unwrap();
    assert_eq!(
        title_event(&reopened.snapshot())
            .get("status")
            .and_then(Value::as_str),
        Some("cancelled")
    );
    let current = agent.sessions().unwrap();
    assert_eq!(current.id(), next_id);
    assert!(
        current.snapshot().events.iter().all(|event| {
            event.get("operation").and_then(Value::as_str) != Some("session_title")
        })
    );
}

#[test]
fn a_title_worker_panic_is_recorded_and_reported() {
    let mut fixture = Fixture::new(TitleOutcome::Panic);
    fixture.gate.release();
    let agent = fixture.agent.as_mut().unwrap();
    assert!(
        agent
            .run_single_turn("First request", &mut |_| Ok(()))
            .unwrap_err()
            .contains("Session title worker failed")
    );
    let handle = agent.sessions().unwrap();
    let saved = handle.store().read(&handle.id()).unwrap();
    assert_eq!(
        title_event(&saved).get("status").and_then(Value::as_str),
        Some("failed")
    );
    assert_eq!(title_event(&saved).get("input_tokens"), Some(&Value::Null));
}

#[test]
fn title_save_errors_reach_the_owner_and_a_retry_keeps_one_event() {
    let mut fixture = Fixture::new(TitleOutcome::Failed(Some(partial_usage())));
    let agent = fixture.agent.as_mut().unwrap();
    agent.prepare_turn("First request").unwrap();
    fixture.gate.wait_for(|state| state.entered);
    let handle = agent.sessions().unwrap();
    let journal = handle
        .store()
        .session_directory(&handle.id())
        .join("session.jsonl");
    let retained = journal.with_extension("retained");
    std::fs::rename(&journal, &retained).unwrap();
    std::fs::create_dir(&journal).unwrap();
    agent.cancellation().cancel();
    let error = agent
        .run_single_turn("First request", &mut |_| Ok(()))
        .unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    assert!(error.contains("regular files"), "{error}");
    std::fs::remove_dir(&journal).unwrap();
    std::fs::rename(&retained, &journal).unwrap();
    handle.flush().unwrap();
    let saved = handle.store().read(&handle.id()).unwrap();
    assert_eq!(
        title_event(&saved)
            .get("input_tokens")
            .and_then(Value::as_usize),
        Some(17)
    );
}
