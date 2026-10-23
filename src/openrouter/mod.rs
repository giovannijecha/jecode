mod capacity;
mod completion;
pub mod endpoints;
mod failure;
mod headers;
mod models;
pub mod routing;
mod served;
mod stream;
mod transport;

pub use capacity::Limits;
pub use endpoints::Endpoints;
pub use failure::{Failure, Kind as FailureKind};
pub use models::Model;
pub use routing::{Router, Routing};
pub use served::Charges;
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
    /// The model's input modalities from the catalog; None when unknown.
    inputs: Option<Vec<String>>,
    router: Router,
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
    /// The serving provider's name, when the response reports it.
    pub provider: Option<String>,
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

    /// A loopback endpoint starts with unbounded limits; OpenRouter limits
    /// are looked up per model.
    fn initial_limits(&self) -> Option<Option<Limits>> {
        self.base_url
            .starts_with("http://127.0.0.1:")
            .then_some(Some(Limits {
                context: usize::MAX,
                output: None,
            }))
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
            limits: api.initial_limits(),
            api,
            model,
            effort: Effort::Default,
            inputs: None,
            router: Router::new(Routing::Default),
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn inputs(&self) -> crate::attachments::provider::Inputs {
        crate::attachments::provider::Inputs {
            image: self.accepts("image"),
            file: self.accepts("file"),
        }
    }

    /// Whether the model takes an input modality such as "image" or "file";
    /// None until the catalog has been read or when the model is not listed.
    pub fn accepts(&self, modality: &str) -> Option<bool> {
        self.inputs
            .as_ref()
            .map(|inputs| inputs.iter().any(|input| input == modality))
    }

    #[cfg(test)]
    pub fn fixture_inputs(&mut self, inputs: &[&str]) {
        self.inputs = Some(inputs.iter().map(|input| (*input).to_owned()).collect());
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

    /// A different model drops endpoint metadata and never keeps an
    /// endpoint pin, which names one model's endpoint.
    pub fn set_model(&mut self, model: String) -> Result<(), String> {
        validate_model(&model)?;
        if model != self.model {
            self.inputs = None;
            let session = self.router.session.take();
            self.router = Router::new(match &self.router.routing {
                Routing::Endpoint(_) => Routing::Auto,
                routing => routing.clone(),
            });
            self.router.session = session;
            self.limits = self.api.initial_limits();
        }
        self.model = model;
        Ok(())
    }

    pub fn routing(&self) -> &Routing {
        &self.router.routing
    }

    pub fn router(&self) -> &Router {
        &self.router
    }

    pub fn set_routing(&mut self, routing: Routing) {
        self.router.routing = routing;
    }

    /// Keeps another client's routing and metadata, as when only the key changes.
    pub fn set_router(&mut self, router: Router) {
        self.router = router;
    }

    /// The conversation identifier Auto sends as `session_id`.
    pub fn set_conversation(&mut self, id: Option<String>) {
        self.router.session = id;
    }

    /// Endpoint metadata fetched elsewhere, such as by the routing selector.
    pub fn offer_endpoints(&mut self, endpoints: Endpoints) {
        if endpoints.model == self.model {
            self.router.offer(endpoints);
        }
    }

    /// Fetches endpoint metadata when routing needs it and the last listing
    /// is stale. A failure leaves Auto to native routing and a pin to its tag.
    pub fn refresh_endpoints(&mut self) {
        let now = std::time::Instant::now();
        if !self.router.needs_refresh(&self.model, now) {
            return;
        }
        match self.api.endpoints(&self.model) {
            Ok(endpoints) => self.router.offer(endpoints),
            Err(_) => self.router.failed(&self.model, std::time::Instant::now()),
        }
    }

    /// Whether the effort asks endpoints for reasoning support.
    pub fn reasoning(&self) -> bool {
        self.effort.reasons()
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
            }
            if let Some(tokens) = output_tokens {
                fields.insert("max_completion_tokens".into(), Value::number(tokens));
            }
            let now = std::time::Instant::now();
            for (key, value) in self.router.fields(&self.model, self.reasoning(), now) {
                fields.insert(key.into(), value);
            }
        }
        if let Some(reasoning) = self.effort.request()
            && let Value::Object(fields) = &mut body
        {
            fields.insert("reasoning".into(), reasoning);
        }
        if crate::attachments::provider::has_files(messages)
            && let Value::Object(fields) = &mut body
        {
            fields.insert(
                "plugins".into(),
                crate::attachments::provider::plugins(self.accepts("file")),
            );
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
                failure.charges = stream.reported(None).or(failure.charges);
                self.pin_hint(self.api.sanitize_failure(failure))
            })?;
        let parsed = crate::json::parse(&response).ok();
        let charges = stream.reported(parsed.as_ref());
        if let Some(value) = &parsed
            && api_error(value).is_some()
        {
            let mut failure = self.api.failure(value, 200);
            failure.charges = charges;
            return Err(self.pin_hint(failure));
        }
        stream.finish(&response).map_err(|error| {
            let mut failure = Failure::response(self.redact(&error));
            failure.charges = charges;
            failure
        })
    }

    /// A pinned request has no fallback, so availability failures say how to change it.
    fn pin_hint(&self, mut failure: Failure) -> Failure {
        if let Routing::Endpoint(tag) = &self.router.routing
            && matches!(failure.kind, FailureKind::Temporary | FailureKind::Terminal)
            && failure.message != "Operation cancelled"
        {
            failure.message = format!(
                "{} Pinned endpoint {tag} has no fallbacks; use /routing to choose Auto or another endpoint.",
                failure.message.trim_end()
            );
        }
        failure
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
