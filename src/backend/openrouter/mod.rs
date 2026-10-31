pub(crate) mod annotations;
pub(crate) mod attachments;
mod backend;
mod cache;
mod calls;
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

pub use crate::backend::{
    Charges, Completion, Failure, FailureKind, Limits, ToolCall, Update, validate_model,
};
pub use endpoints::Endpoints;
pub use routing::{Router, Routing};

use crate::cancel::Cancellation;
use crate::effort::Effort;
use crate::json::Value;
use crate::redact::Redactor;
use crate::tools;

const BASE_URL: &str = "https://openrouter.ai/api/v1";
pub(super) const CREDENTIAL_ENV: &[&str] = &["OPENROUTER_API_KEY"];

#[derive(Clone)]
pub struct Api {
    api_key: String,
    account: String,
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
    delegation: bool,
}

pub fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > 512 || !key.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err("The OpenRouter API key must be a nonempty token without whitespace".into());
    }
    Ok(())
}

impl Api {
    pub fn new(api_key: String) -> Result<Self, String> {
        validate_key(&api_key)?;
        let cancellation = Cancellation::default();
        let account = crate::auth::key_scope(&api_key, &cancellation)?;
        Ok(Self {
            api_key,
            account,
            base_url: BASE_URL.into(),
            cancellation,
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
        self.account = crate::auth::key_scope(&key, &self.cancellation)?;
        self.api_key = key;
        Ok(self)
    }

    #[cfg(test)]
    pub fn fixture(endpoint: &str) -> Self {
        assert!(endpoint.starts_with("http://127.0.0.1:"));
        static ACCOUNT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        Self {
            api_key: "isolated-fixture-key".into(),
            account: ACCOUNT
                .get_or_init(|| {
                    crate::auth::key_scope("isolated-fixture-key", &Cancellation::default())
                        .unwrap()
                })
                .clone(),
            base_url: endpoint.trim_end_matches("/chat/completions").into(),
            cancellation: Cancellation::default(),
        }
    }
}

impl OpenRouter {
    #[cfg(test)]
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
            delegation: true,
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn inputs(&self) -> crate::attachments::Inputs {
        crate::attachments::Inputs {
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
    #[cfg(test)]
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

    /// The conversation identifier sent as `session_id` outside endpoint pins.
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

    #[cfg(test)]
    pub fn sample(
        &self,
        messages: &[Value],
        with_tools: bool,
        output_tokens: Option<usize>,
        emit: &mut impl FnMut(Update) -> Result<(), String>,
    ) -> Result<Completion, Failure> {
        self.sample_owned(
            messages.to_vec(),
            self.effort,
            with_tools,
            false,
            output_tokens,
            emit,
        )
    }

    fn sample_owned(
        &self,
        mut messages: Vec<Value>,
        effort: Effort,
        with_tools: bool,
        request_state: bool,
        output_tokens: Option<usize>,
        emit: &mut impl FnMut(Update) -> Result<(), String>,
    ) -> Result<Completion, Failure> {
        let has_files = attachments::has_files(&messages);
        let claude_cache = with_tools
            && self
                .model
                .strip_prefix('~')
                .unwrap_or(&self.model)
                .starts_with("anthropic/claude-");
        let explicit_cache = with_tools && (claude_cache || cache::supports_openai(&self.model));
        if explicit_cache && request_state && messages.len() > 1 {
            let durable = messages.len() - 1;
            cache::mark_recent(&mut messages[..durable]);
        }
        let mut body = Value::object([
            ("model", Value::string(&self.model)),
            ("messages", Value::Array(messages)),
            ("stream", Value::Bool(true)),
        ]);
        if let Value::Object(fields) = &mut body {
            if with_tools {
                let definitions = tools::definitions_for(self.delegation);
                let wrapped = definitions
                    .as_array()
                    .unwrap_or(&[])
                    .iter()
                    .cloned()
                    .map(|definition| {
                        Value::object([
                            ("type", Value::string("function")),
                            ("function", definition),
                        ])
                    })
                    .collect();
                fields.insert("tools".into(), Value::Array(wrapped));
                // OpenRouter supports automatic top-level breakpoints for Claude.
                if claude_cache && !request_state {
                    fields.insert(
                        "cache_control".into(),
                        Value::object([("type", Value::string("ephemeral"))]),
                    );
                }
            }
            if let Some(tokens) = output_tokens {
                fields.insert("max_completion_tokens".into(), Value::number(tokens));
            }
            let now = std::time::Instant::now();
            for (key, value) in self.router.fields(&self.model, effort.reasons(), now) {
                fields.insert(key.into(), value);
            }
        }
        if let Some(reasoning) = reasoning_request(effort)
            && let Value::Object(fields) = &mut body
        {
            fields.insert("reasoning".into(), reasoning);
        }
        if has_files && let Value::Object(fields) = &mut body {
            fields.insert("plugins".into(), attachments::plugins(self.accepts("file")));
        }
        let body = body.encode();
        let mut stream = stream::Stream::default();
        emit(Update::Waiting {
            request_bytes: body.len(),
        })
        .map_err(Failure::from)?;
        let response = self
            .api
            .raw("POST", "/chat/completions", Some(&body), &mut |bytes| {
                stream.feed(bytes, emit)
            })
            .map_err(|error| {
                let mut failure = stream.failure.clone().unwrap_or_else(|| error.clone());
                failure.retry_after = failure.retry_after.or(error.retry_after);
                failure.charges = stream.reported(None).map(Box::new).or(failure.charges);
                self.pin_hint(self.api.sanitize_failure(failure))
            })?;
        let parsed = crate::json::parse(&response).ok();
        let charges = stream.reported(parsed.as_ref()).map(Box::new);
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
            if let Some(diagnostic) = &mut failure.diagnostic {
                *diagnostic = format!(
                    "{} Pinned endpoint {tag} has no fallbacks; use /routing to choose Auto or another endpoint.",
                    diagnostic.trim_end()
                );
            }
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

fn reasoning_request(effort: Effort) -> Option<Value> {
    (effort != Effort::Default).then(|| Value::object([("effort", Value::string(effort.name()))]))
}

#[cfg(test)]
mod tests;
