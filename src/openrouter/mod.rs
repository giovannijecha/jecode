mod capacity;
mod completion;
mod failure;
mod headers;
mod models;
mod stream;
mod transport;

pub use capacity::Limits;
pub use failure::{Failure, Kind as FailureKind};
pub use models::Model;
pub use stream::Update;

use crate::cancel::Cancellation;
use crate::effort::Effort;
use crate::json::Value;
use crate::redact::Redactor;
use crate::tools;

const BASE_URL: &str = "https://openrouter.ai/api/v1";

#[derive(Clone)]
pub struct Api {
    api_key: String,
    base_url: String,
    cancellation: Cancellation,
}

pub struct OpenRouter {
    api: Api,
    model: String,
    effort: Effort,
    limits: Option<Option<Limits>>,
    summary_json: bool,
}

pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

pub struct Completion {
    pub message: Value,
    pub text: String,
    pub calls: Vec<ToolCall>,
    pub usage: Option<Value>,
}

pub fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > 512 || !key.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err("The OpenRouter API key must be a nonempty token without whitespace".into());
    }
    Ok(())
}

pub fn validate_model(model: &str) -> Result<(), String> {
    if model.is_empty()
        || model.len() > 256
        || model.chars().any(char::is_whitespace)
        || model.chars().any(char::is_control)
    {
        return Err("The model identifier must be 1-256 characters without whitespace".into());
    }
    Ok(())
}

impl Api {
    pub fn new(api_key: String) -> Result<Self, String> {
        validate_key(&api_key)?;
        Ok(Self {
            api_key,
            base_url: BASE_URL.into(),
            cancellation: Cancellation::default(),
        })
    }

    pub fn redact(&self, text: &str) -> String {
        self.redactor().text(text)
    }

    pub fn redactor(&self) -> Redactor {
        Redactor::new(self.api_key.clone())
    }

    pub fn check_key(&self) -> Result<(), String> {
        let response = self.request("GET", "/key", None)?;
        if !matches!(response.get("data"), Some(Value::Object(_))) {
            return Err("OpenRouter returned an invalid key response".into());
        }
        Ok(())
    }

    pub fn with_cancellation(mut self, cancellation: Cancellation) -> Self {
        self.cancellation = cancellation;
        self
    }
    pub fn with_key(mut self, key: String) -> Result<Self, String> {
        validate_key(&key)?;
        self.api_key = key;
        Ok(self)
    }

    #[cfg(test)]
    pub fn fixture(endpoint: &str) -> Self {
        assert!(endpoint.starts_with("http://127.0.0.1:"));
        Self {
            api_key: "isolated-fixture-key".into(),
            base_url: endpoint.trim_end_matches("/chat/completions").into(),
            cancellation: Cancellation::default(),
        }
    }
}

impl OpenRouter {
    pub fn new(api_key: String, model: String) -> Result<Self, String> {
        Self::with_api(Api::new(api_key)?, model)
    }

    pub fn with_api(api: Api, model: String) -> Result<Self, String> {
        validate_model(&model)?;
        Ok(Self {
            limits: if api.base_url.starts_with("http://127.0.0.1:") {
                Some(Some(Limits {
                    context: usize::MAX,
                    output: None,
                }))
            } else {
                None
            },
            api,
            model,
            effort: Effort::Default,
            summary_json: false,
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn api(&self) -> Api {
        self.api.clone()
    }

    pub fn effort(&self) -> Effort {
        self.effort
    }

    pub fn set_effort(&mut self, effort: Effort) {
        self.effort = effort;
    }

    pub fn set_model(&mut self, model: String) -> Result<(), String> {
        validate_model(&model)?;
        if model != self.model {
            self.summary_json = false;
            self.limits = if self.api.base_url.starts_with("http://127.0.0.1:") {
                Some(Some(Limits {
                    context: usize::MAX,
                    output: None,
                }))
            } else {
                None
            };
        }
        self.model = model;
        Ok(())
    }

    pub fn redact(&self, text: &str) -> String {
        self.api.redact(text)
    }

    pub fn redactor(&self) -> Redactor {
        self.api.redactor()
    }

    pub fn set_cancellation(&mut self, cancellation: Cancellation) {
        self.api.cancellation = cancellation;
    }

    #[cfg(test)]
    pub fn complete(&self, messages: &[Value]) -> Result<Completion, String> {
        self.complete_stream(messages, &mut |_| Ok(()))
    }

    #[cfg(test)]
    pub fn complete_stream(
        &self,
        messages: &[Value],
        emit: &mut impl FnMut(Update) -> Result<(), String>,
    ) -> Result<Completion, String> {
        self.sample(messages, true, None, emit)
            .map_err(|failure| failure.message)
    }

    pub fn sample(
        &self,
        messages: &[Value],
        with_tools: bool,
        output_tokens: Option<usize>,
        emit: &mut impl FnMut(Update) -> Result<(), String>,
    ) -> Result<Completion, Failure> {
        let mut body = Value::object([
            ("model", Value::string(&self.model)),
            ("messages", Value::Array(messages.to_vec())),
            ("stream", Value::Bool(true)),
        ]);
        if let Value::Object(fields) = &mut body {
            if with_tools {
                fields.insert("tools".into(), tools::definitions());
            } else if self.summary_json {
                fields.insert(
                    "response_format".into(),
                    Value::object([("type", Value::string("json_object"))]),
                );
            }
            if let Some(tokens) = output_tokens {
                fields.insert("max_completion_tokens".into(), Value::number(tokens));
            }
        }
        if let Some(reasoning) = self.effort.request()
            && let Value::Object(fields) = &mut body
        {
            fields.insert("reasoning".into(), reasoning);
        }
        let body = body.encode();
        let mut stream = stream::Stream::default();
        let response = self
            .api
            .raw("POST", "/chat/completions", Some(&body), &mut |bytes| {
                stream.feed(bytes, emit)
            })
            .map_err(|error| {
                let mut failure = stream.failure.clone().unwrap_or_else(|| error.clone());
                failure.retry_after = failure.retry_after.or(error.retry_after);
                self.api.sanitize_failure(failure)
            })?;
        if let Ok(value) = crate::json::parse(&response)
            && api_error(&value).is_some()
        {
            return Err(self.api.failure(&value, 200));
        }
        stream
            .finish(&response)
            .map_err(|error| Failure::response(self.redact(&error)))
    }

    #[cfg(test)]
    pub fn fixture(endpoint: String) -> Self {
        Self::with_api(Api::fixture(&endpoint), "fixture/model".into()).unwrap()
    }
}

fn api_error(value: &Value) -> Option<&str> {
    value.get("error").and_then(|error| {
        error
            .as_str()
            .or_else(|| error.get("message").and_then(Value::as_str))
    })
}

#[cfg(test)]
mod tests;
