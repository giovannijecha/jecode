use super::*;
use crate::{
    attachments::{Inputs, Pool},
    backend::{Backend, Completion, Failure, Limits, Service, Services, ToolCall, Update},
    cancel::Cancellation,
    effort::Effort,
    events::Event,
    redact::Redactor,
    test_support::Directory,
};
use std::{
    any::Any,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
struct Shared {
    release_child: AtomicBool,
    main_requests: AtomicUsize,
    child_requests: AtomicUsize,
    saw_return: AtomicBool,
    child_has_delegation: AtomicBool,
}

#[derive(Clone)]
struct MockService(Arc<Shared>);

struct MockBackend {
    shared: Arc<Shared>,
    model: String,
    effort: Effort,
    delegation: bool,
}

impl Service for MockService {
    fn id(&self) -> &'static str {
        "fixture"
    }
    fn with_credential(&self, _: String) -> Result<Arc<dyn Service>, String> {
        Ok(Arc::new(self.clone()))
    }
    fn with_cancellation(&self, _: Cancellation) -> Arc<dyn Service> {
        Arc::new(self.clone())
    }
    fn catalog(&self) -> Result<Vec<crate::backend::Model>, String> {
        Ok(vec![])
    }
    fn models(&self, _: &str) -> Result<Vec<crate::backend::Model>, String> {
        Ok(vec![])
    }
    fn check_credential(&self) -> Result<(), String> {
        Ok(())
    }
    fn redactor(&self) -> Redactor {
        Redactor::empty()
    }
    fn create(&self, model: String) -> Result<Box<dyn Backend>, String> {
        Ok(Box::new(MockBackend {
            shared: self.0.clone(),
            model,
            effort: Effort::Default,
            delegation: true,
        }))
    }
}

impl Backend for MockBackend {
    fn id(&self) -> &'static str {
        "fixture"
    }
    fn model(&self) -> &str {
        &self.model
    }
    fn effort(&self) -> Effort {
        self.effort
    }
    fn set_effort(&mut self, effort: Effort) {
        self.effort = effort;
    }
    fn inputs(&self) -> Inputs {
        Inputs::default()
    }
    fn limits(&mut self) -> Result<Option<Limits>, Failure> {
        Ok(None)
    }
    fn known_limits(&self) -> Option<Limits> {
        None
    }
    fn sample(
        &self,
        messages: &[Value],
        _: bool,
        _: Option<usize>,
        _: Option<&Pool>,
        emit: &mut dyn FnMut(Update) -> Result<(), String>,
    ) -> Result<Completion, Failure> {
        emit(Update::Waiting { request_bytes: 1 })?;
        let child = messages.iter().any(|message| {
            message.get("role").and_then(Value::as_str) == Some("user")
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|text| text.starts_with("Assignment v"))
        });
        if child {
            self.shared.child_requests.fetch_add(1, Ordering::SeqCst);
            self.shared
                .child_has_delegation
                .store(self.delegation, Ordering::SeqCst);
            while !self.shared.release_child.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            return Ok(answer("Child completed the assigned check", vec![]));
        }
        let turn = self.shared.main_requests.fetch_add(1, Ordering::SeqCst);
        match turn {
            0 => Ok(answer(
                "Delegating.",
                vec![ToolCall {
                    id: "create".into(),
                    name: "agent".into(),
                    arguments: Value::object([
                        ("action", Value::string("create")),
                        (
                            "tasks",
                            Value::Array(vec![Value::object([
                                ("title", Value::string("Check the project")),
                                ("instructions", Value::string("Inspect and report")),
                                ("workspace", Value::string("shared")),
                            ])]),
                        ),
                    ])
                    .encode(),
                }],
            )),
            1 => Ok(answer("I will review the result when it returns.", vec![])),
            2 => {
                self.shared.saw_return.store(
                    messages
                        .first()
                        .and_then(|message| message.get("content"))
                        .and_then(Value::as_str)
                        .is_some_and(|text| text.contains("Child completed the assigned check")),
                    Ordering::SeqCst,
                );
                Ok(answer(
                    "Reviewing the returned work.",
                    vec![ToolCall {
                        id: "review".into(),
                        name: "agent".into(),
                        arguments: Value::object([
                            ("action", Value::string("review")),
                            ("id", Value::string("a1")),
                            ("report_version", Value::number(1)),
                            ("note", Value::string("Confirmed in fixture")),
                        ])
                        .encode(),
                    }],
                ))
            }
            _ => Ok(answer("Reviewed.", vec![])),
        }
    }
    fn redactor(&self) -> Redactor {
        Redactor::empty()
    }
    fn set_cancellation(&mut self, _: Cancellation) {}
    fn set_conversation(&mut self, _: Option<String>) {}
    fn set_delegation(&mut self, enabled: bool) {
        self.delegation = enabled;
    }
    fn services(&self) -> Services {
        Services::from_service(MockService(self.shared.clone()))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn answer(text: &str, calls: Vec<ToolCall>) -> Completion {
    Completion {
        message: Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string(text)),
            (
                "tool_calls",
                Value::Array(calls.iter().map(ToolCall::value).collect()),
            ),
        ]),
        text: text.into(),
        calls,
        usage: None,
        charges: None,
        postprocess_error: None,
    }
}

#[test]
fn live_return_continues_main_model_without_new_user_message() {
    let project = Directory::new();
    let home = Directory::new();
    let shared = Arc::new(Shared::default());
    let backend = MockBackend {
        shared: shared.clone(),
        model: "fixture-model".into(),
        effort: Effort::Default,
        delegation: true,
    };
    let mut agent = Agent::new(backend, Tools::new(project.path()).unwrap());
    agent.enable_sessions(home.path()).unwrap();
    agent
        .run_turn("Delegate the check", &mut |_| Ok(()))
        .unwrap();
    assert_eq!(shared.main_requests.load(Ordering::SeqCst), 2);
    let record = agent.collaboration().snapshot().remove(0);
    assert_eq!(
        record.get("state").and_then(Value::as_str),
        Some("running"),
        "{}",
        record.encode()
    );
    shared.release_child.store(true, Ordering::SeqCst);
    agent
        .collaboration()
        .wait(Some("a1"), Duration::from_secs(5), &Cancellation::default())
        .unwrap();
    let user_messages_before = agent
        .archive()
        .messages
        .lock()
        .unwrap()
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .count();
    let mut receipts = 0;
    assert!(
        agent
            .continue_collaboration(&mut |event| {
                if matches!(event, Event::Collaboration { .. }) {
                    receipts += 1;
                }
                Ok(())
            })
            .unwrap()
    );
    assert!(!agent.continue_collaboration(&mut |_| Ok(())).unwrap());
    let user_messages_after = agent
        .archive()
        .messages
        .lock()
        .unwrap()
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .count();
    assert_eq!(user_messages_after, user_messages_before);
    assert_eq!(shared.main_requests.load(Ordering::SeqCst), 4);
    assert!(shared.saw_return.load(Ordering::SeqCst));
    assert!(!shared.child_has_delegation.load(Ordering::SeqCst));
    assert_eq!(
        agent.collaboration().snapshot()[0].get("reviewed"),
        Some(&Value::Bool(true))
    );
    assert!(receipts >= 1);
}
