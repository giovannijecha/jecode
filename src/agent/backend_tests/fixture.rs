use crate::{
    attachments::{Inputs, Pool},
    backend::{
        self, Backend, Charges, Completion, Failure, Limits, Model, Service, Services, Update,
        Usage,
    },
    cancel::Cancellation,
    conversation::{self, Origin, ToolCall},
    effort::Effort,
    json::Value,
    redact::Redactor,
};
use std::{
    any::Any,
    collections::VecDeque,
    sync::{Arc, Mutex},
};

pub struct Reply {
    pub text: &'static str,
    pub calls: Vec<ToolCall>,
}

impl Reply {
    pub fn text(text: &'static str) -> Self {
        Self {
            text,
            calls: vec![],
        }
    }
}

#[derive(Clone)]
pub struct Account {
    pub id: &'static str,
    replies: Arc<Mutex<VecDeque<Reply>>>,
    pub requests: Arc<Mutex<Vec<Value>>>,
}

impl Account {
    pub fn new(id: &'static str, replies: Vec<Reply>) -> Self {
        Self {
            id,
            replies: Arc::new(Mutex::new(replies.into())),
            requests: Arc::default(),
        }
    }
    pub fn client(&self) -> Client {
        Client {
            account: self.clone(),
            model: "shared-model-name".into(),
            effort: Effort::Default,
            fail_prepare: false,
        }
    }
    pub fn services(&self) -> Services {
        Services::from_service(self.clone())
    }
}

impl Service for Account {
    fn id(&self) -> &'static str {
        self.id
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
        self.catalog()
    }
    fn check_credential(&self) -> Result<(), String> {
        Ok(())
    }
    fn redactor(&self) -> Redactor {
        Redactor::empty()
    }
    fn create(&self, model: String) -> Result<Box<dyn Backend>, String> {
        backend::validate_model(&model)?;
        Ok(Box::new(Client {
            model,
            ..self.client()
        }))
    }
}

pub struct Client {
    account: Account,
    model: String,
    effort: Effort,
    pub fail_prepare: bool,
}

impl Backend for Client {
    fn id(&self) -> &'static str {
        self.account.id
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
        with_tools: bool,
        _: Option<usize>,
        _: Option<&Pool>,
        emit: &mut dyn FnMut(Update) -> Result<(), String>,
    ) -> Result<Completion, Failure> {
        let origin = Origin::new(self.id(), self.model());
        // Two deliberately different request protocols, both built from the
        // canonical conversation and catalog rather than another adapter.
        let projected = messages
            .iter()
            .map(|message| conversation::project(message, &origin));
        let request = if self.id() == "alpha" {
            Value::object([
                ("dialog", Value::Array(projected.collect())),
                (
                    "functions",
                    if with_tools {
                        crate::tools::definitions().clone()
                    } else {
                        Value::Null
                    },
                ),
            ])
        } else {
            Value::object([
                (
                    "turns",
                    Value::Array(
                        projected
                            .map(|message| {
                                Value::object([
                                    (
                                        "speaker",
                                        message.get("role").cloned().unwrap_or(Value::Null),
                                    ),
                                    ("payload", message),
                                ])
                            })
                            .collect(),
                    ),
                ),
                (
                    "operations",
                    if with_tools {
                        crate::tools::definitions().clone()
                    } else {
                        Value::Null
                    },
                ),
            ])
        };
        let request_bytes = request.encode().len();
        self.account.requests.lock().unwrap().push(request);
        emit(Update::Waiting { request_bytes })?;
        let reply = self
            .account
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or("Unexpected fixture request")?;
        if !reply.text.is_empty() {
            emit(Update::TextDelta(reply.text.into()))?;
        }
        let mut message = Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string(reply.text)),
            (
                "tool_calls",
                Value::Array(reply.calls.iter().map(ToolCall::value).collect()),
            ),
            (
                "native_state",
                Value::string(format!("{}-private-state", self.id())),
            ),
        ]);
        conversation::stamp(&mut message, &origin);
        Ok(Completion {
            message,
            text: reply.text.into(),
            calls: reply.calls,
            usage: Some(Usage {
                input_tokens: Some(73),
                output_tokens: Some(4),
                cached_tokens: None,
                cache_write_tokens: None,
            }),
            charges: Some(Charges {
                requests: 1,
                ..Default::default()
            }),
            postprocess_error: None,
        })
    }
    fn redactor(&self) -> Redactor {
        Redactor::empty()
    }
    fn set_cancellation(&mut self, _: Cancellation) {}
    fn set_conversation(&mut self, _: Option<String>) {}
    fn services(&self) -> Services {
        self.account.services()
    }
    fn prepare(&mut self) -> Result<(), String> {
        if self.fail_prepare {
            Err("Fixture account unavailable".into())
        } else {
            Ok(())
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
