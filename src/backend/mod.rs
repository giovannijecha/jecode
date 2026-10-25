//! Model inference and account services independent of a provider's wire format.

mod charges;
mod failure;
pub(crate) mod openai;
pub(crate) mod openrouter;
pub(crate) use charges::{price, quantity};

use crate::attachments::{Inputs, Pool};
use crate::cancel::Cancellation;
pub use crate::conversation::ToolCall;
use crate::effort::Effort;
use crate::json::Value;
use crate::redact::Redactor;
use std::any::Any;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub context: usize,
    pub output: Option<usize>,
}

impl Limits {
    pub fn output_allowance(self, input_tokens: usize) -> usize {
        self.output
            .unwrap_or(self.context)
            .min(self.context.saturating_sub(input_tokens))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FailureKind {
    Temporary,
    Empty,
    Context,
    Length,
    Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureReason {
    RateLimited,
    RejectedKey,
}

#[derive(Clone, Debug)]
pub struct Failure {
    pub kind: FailureKind,
    pub reason: Option<FailureReason>,
    pub message: String,
    /// Diagnostic without a consumer-specific next-step hint, when one exists.
    pub diagnostic: Option<String>,
    pub retry_after: Option<Duration>,
    pub charges: Option<Box<Charges>>,
}

pub enum Update {
    /// Request construction is complete and the backend is about to send it.
    Waiting {
        request_bytes: usize,
    },
    Reasoning,
    /// Newly generated text, appended to the current attempt's response.
    TextDelta(String),
    Working,
}

pub struct Completion {
    /// Canonical assistant message for history; provider extras are stamped by the adapter.
    pub message: Value,
    pub text: String,
    pub calls: Vec<ToolCall>,
    pub usage: Option<Usage>,
    pub charges: Option<Charges>,
    /// Local adapter normalization failed after a usable answer arrived.
    pub postprocess_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: Option<usize>,
    pub output_tokens: Option<usize>,
    pub cached_tokens: Option<usize>,
    pub cache_write_tokens: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Charges {
    pub providers: Vec<String>,
    pub requests: usize,
    pub priced: usize,
    pub cost: f64,
    pub cached: usize,
    pub cache_written: usize,
    pub byok: bool,
    pub upstream: f64,
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

pub fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err("Backend identifier must use lowercase letters, digits or hyphens".into());
    }
    Ok(())
}

#[derive(Clone)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub prompt_price: Option<f64>,
    pub completion_price: Option<f64>,
    pub efforts: Vec<Effort>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Routing {
    Auto,
    Endpoint(String),
    Default,
}

impl Routing {
    pub fn parse(value: &Value) -> Option<Self> {
        match value.get("mode")?.as_str()? {
            "auto" => Some(Self::Auto),
            "default" => Some(Self::Default),
            "endpoint" => value
                .get("endpoint")
                .and_then(Value::as_str)
                .filter(|tag| {
                    !tag.is_empty()
                        && tag.len() <= 128
                        && tag.bytes().all(|byte| byte.is_ascii_graphic())
                })
                .map(|tag| Self::Endpoint(tag.into())),
            _ => None,
        }
    }

    pub fn value(&self) -> Value {
        match self {
            Self::Auto => Value::object([("mode", Value::string("auto"))]),
            Self::Default => Value::object([("mode", Value::string("default"))]),
            Self::Endpoint(tag) => Value::object([
                ("mode", Value::string("endpoint")),
                ("endpoint", Value::string(tag)),
            ]),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Auto => "Auto — Best balance".into(),
            Self::Endpoint(tag) => format!("Endpoint {tag}"),
            Self::Default => "Backend default".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Endpoint {
    pub tag: String,
    pub provider: Option<String>,
    pub prompt: Option<f64>,
    pub completion: Option<f64>,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
    pub discount: Option<f64>,
    pub context: Option<usize>,
    pub output: Option<usize>,
    pub quantization: Option<String>,
    pub tier: Option<String>,
    pub status: Option<i64>,
    pub uptime: Option<f64>,
    pub latency: Option<f64>,
    pub throughput: Option<f64>,
    pub requests: Option<u64>,
    pub parameters: Option<Vec<String>>,
}

impl Endpoint {
    pub fn supports(&self, parameter: &str) -> Option<bool> {
        self.parameters
            .as_ref()
            .map(|parameters| parameters.iter().any(|value| value == parameter))
    }
}

#[derive(Clone, Debug)]
pub struct Endpoints {
    pub model: String,
    pub list: Vec<Endpoint>,
    pub fetched: Instant,
}

impl Endpoints {
    pub fn find(&self, tag: &str) -> Option<&Endpoint> {
        self.list.iter().find(|endpoint| endpoint.tag == tag)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub routing: Routing,
    pub name: String,
    /// Every fact on one line, for plain output.
    pub description: String,
    /// The measured endpoint's comparable figures, for a table.
    pub metrics: Option<Metrics>,
    /// The facts a table leaves out of its columns.
    pub facts: String,
    pub help: String,
    pub problem: Option<String>,
}

/// Prices per million tokens after discounts, median throughput in tokens
/// per second and median seconds to the first token.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Metrics {
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub throughput: Option<f64>,
    pub latency: Option<f64>,
}

/// Every backend accepts the same canonical transcript and tool catalog.
pub trait Backend: Send {
    fn id(&self) -> &'static str;
    fn account(&self) -> Option<String> {
        None
    }
    fn model(&self) -> &str;
    fn effort(&self) -> Effort;
    fn set_effort(&mut self, effort: Effort);
    fn inputs(&self) -> Inputs;
    fn limits(&mut self) -> Result<Option<Limits>, Failure>;
    fn known_limits(&self) -> Option<Limits>;
    fn sample(
        &self,
        messages: &[Value],
        with_tools: bool,
        output_tokens: Option<usize>,
        pool: Option<&Pool>,
        emit: &mut dyn FnMut(Update) -> Result<(), String>,
    ) -> Result<Completion, Failure>;
    fn redactor(&self) -> Redactor;
    fn set_cancellation(&mut self, cancellation: Cancellation);
    fn set_conversation(&mut self, id: Option<String>);
    fn services(&self) -> Services;
    fn prepare(&mut self) -> Result<(), String> {
        Ok(())
    }
    /// Prepares routing and returns the limits for the next inference turn.
    fn prepare_turn(&mut self) -> Result<Option<Limits>, Failure> {
        self.prepare().map_err(Failure::from)?;
        self.limits()
    }
    fn routing(&self) -> Routing {
        Routing::Default
    }
    fn set_routing(&mut self, _routing: Routing) {}
    fn offer_endpoints(&mut self, _endpoints: Endpoints) {}
    fn cached_endpoints(&self) -> Option<Endpoints> {
        None
    }
    fn clone_routing_from(&mut self, _previous: &dyn Backend) {}
    fn as_any(&self) -> &dyn Any;
    #[cfg(test)]
    fn fixture_limits(&mut self, _context: usize, _output: Option<usize>) {}
    #[cfg(test)]
    fn fixture_unknown_limits(&mut self) {}
}

pub struct BackendInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub key_url: Option<&'static str>,
    pub credential_env: &'static [&'static str],
}
static AVAILABLE: [BackendInfo; 2] = [
    BackendInfo {
        id: "openrouter",
        name: "OpenRouter",
        key_url: Some("https://openrouter.ai/settings/keys"),
        credential_env: openrouter::CREDENTIAL_ENV,
    },
    BackendInfo {
        id: "openai",
        name: "OpenAI",
        key_url: None,
        credential_env: openai::CREDENTIAL_ENV,
    },
];
pub fn available() -> &'static [BackendInfo] {
    &AVAILABLE
}

/// Checks credential syntax without contacting the backend.
pub fn validate_credential(id: &str, key: &str) -> Result<(), String> {
    Services::for_backend(id, key.to_owned()).map(|_| ())
}

pub trait Service: Send + Sync {
    fn id(&self) -> &'static str;
    fn account(&self) -> Option<String> {
        None
    }
    /// Whether catalog effort options describe known model capabilities.
    fn validates_effort(&self) -> bool {
        true
    }
    fn with_credential(&self, key: String) -> Result<Arc<dyn Service>, String>;
    fn with_cancellation(&self, cancellation: Cancellation) -> Arc<dyn Service>;
    fn catalog(&self) -> Result<Vec<Model>, String>;
    fn models(&self, query: &str) -> Result<Vec<Model>, String>;
    fn check_credential(&self) -> Result<(), String>;
    fn redactor(&self) -> Redactor;
    fn endpoints(&self, _model: &str) -> Result<Option<Endpoints>, String> {
        Ok(None)
    }
    fn routing_entries(
        &self,
        _endpoints: Option<&Endpoints>,
        _current: &Routing,
        _reasoning: bool,
    ) -> Option<Vec<Entry>> {
        None
    }
    fn create(&self, model: String) -> Result<Box<dyn Backend>, String>;
}

#[derive(Clone)]
pub struct Services {
    inner: Arc<dyn Service>,
}

impl Services {
    pub fn for_backend(id: &str, key: String) -> Result<Self, String> {
        Self::for_credential(&crate::auth::Credential::api_key(id, key))
    }
    pub fn for_credential(credential: &crate::auth::Credential) -> Result<Self, String> {
        match credential.identity.backend.as_str() {
            "openrouter" => Ok(Self::from_service(openrouter::Api::new(
                credential
                    .api_key_material()
                    .ok_or("OpenRouter needs an API key")?
                    .to_owned(),
            )?)),
            "openai" => Ok(Self::from_service(openai::Api::new(
                credential
                    .account_session()
                    .ok_or(
                        "OpenAI needs a ChatGPT account; open Settings > Providers > Credentials",
                    )?
                    .clone(),
            )?)),
            id => Err(format!("Unknown backend: {id}")),
        }
    }
    pub fn from_service(service: impl Service + 'static) -> Self {
        Self {
            inner: Arc::new(service),
        }
    }
    pub fn id(&self) -> &'static str {
        self.inner.id()
    }
    pub fn account(&self) -> Option<String> {
        self.inner.account()
    }
    pub fn validates_effort(&self) -> bool {
        self.inner.validates_effort()
    }
    pub fn with_credential(&self, key: String) -> Result<Self, String> {
        Ok(Self {
            inner: self.inner.with_credential(key)?,
        })
    }
    pub fn with_cancellation(self, cancellation: Cancellation) -> Self {
        Self {
            inner: self.inner.with_cancellation(cancellation),
        }
    }
    pub fn catalog(&self) -> Result<Vec<Model>, String> {
        self.inner.catalog()
    }
    pub fn models(&self, query: &str) -> Result<Vec<Model>, String> {
        self.inner.models(query)
    }
    pub fn check_credential(&self) -> Result<(), String> {
        self.inner.check_credential()
    }
    pub fn redact(&self, text: &str) -> String {
        self.redactor().text(text)
    }
    pub fn redactor(&self) -> Redactor {
        self.inner.redactor()
    }
    #[cfg(test)]
    pub fn fixture(endpoint: &str) -> Self {
        Self::from_service(openrouter::Api::fixture(endpoint))
    }
    pub fn endpoints(&self, model: &str) -> Result<Option<Endpoints>, String> {
        self.inner.endpoints(model)
    }
    pub fn routing_entries(
        &self,
        endpoints: Option<&Endpoints>,
        current: &Routing,
        reasoning: bool,
    ) -> Option<Vec<Entry>> {
        self.inner.routing_entries(endpoints, current, reasoning)
    }
    pub fn create(&self, model: String) -> Result<Box<dyn Backend>, String> {
        self.inner.create(model)
    }
}
