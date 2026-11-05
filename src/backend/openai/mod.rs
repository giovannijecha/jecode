//! ChatGPT plan and API-key inference over OpenAI's Responses API.

#[cfg(test)]
mod agent_tests;
#[cfg(test)]
mod efficiency_live;
#[cfg(test)]
mod live;
mod metadata;
mod socket;
#[cfg(test)]
mod socket_tests;
mod stream;
mod transport;
mod wire;

#[cfg(test)]
mod tests;

use crate::attachments::{Inputs, Pool};
use crate::auth::openai::Session;
use crate::backend::{self, Backend, Service};
use crate::cancel::Cancellation;
use crate::conversation::{self, Origin};
use crate::effort::Effort;
use crate::json::Value;
use crate::redact::Redactor;
use std::any::Any;
use std::sync::{Arc, Mutex};

pub const CREDENTIAL_ENV: &[&str] = &["JECODE_OPENAI_ACCESS_TOKEN"];
pub const API_CREDENTIAL_ENV: &[&str] = &["JECODE_OPENAI_API_KEY"];

#[derive(Clone)]
enum Credential {
    Account(Session),
    ApiKey { key: String, scope: String },
}

#[derive(Clone)]
pub struct Api {
    credential: Credential,
    cancellation: Cancellation,
    base_url: String,
    metadata: Arc<Mutex<Option<metadata::Catalog>>>,
    /// Whether conversations may use the Responses WebSocket before HTTP.
    sockets: bool,
}

pub struct OpenAi {
    api: Api,
    model: String,
    effort: Effort,
    /// The saved conversation, used for prompt cache affinity on both routes.
    /// ChatGPT also sends it as `session-id`.
    conversation: Option<String>,
    delegation: bool,
    channel: Mutex<socket::Channel>,
}

impl Api {
    pub fn new(session: Session) -> Result<Self, String> {
        Ok(Self {
            credential: Credential::Account(session),
            cancellation: Cancellation::default(),
            base_url: "https://api.openai.com/v1".into(),
            metadata: Arc::new(Mutex::new(None)),
            sockets: true,
        })
    }

    pub fn with_api_key(key: String) -> Result<Self, String> {
        if key.is_empty() || key.len() > 512 || !key.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err("The OpenAI API key must be a nonempty token without whitespace".into());
        }
        let cancellation = Cancellation::default();
        let scope = crate::auth::key_scope(&key, &cancellation)?;
        Ok(Self {
            credential: Credential::ApiKey { key, scope },
            cancellation,
            base_url: "https://api.openai.com/v1".into(),
            metadata: Arc::new(Mutex::new(None)),
            sockets: true,
        })
    }

    /// An HTTP-only loopback route; socket fixtures opt in with `with_sockets`.
    #[cfg(test)]
    pub fn fixture(session: Session, endpoint: &str) -> Self {
        assert!(endpoint.starts_with("http://127.0.0.1:"));
        Self {
            credential: Credential::Account(session),
            cancellation: Cancellation::default(),
            base_url: endpoint.into(),
            metadata: Arc::new(Mutex::new(Some(metadata::Catalog {
                models: vec![],
                limits: Default::default(),
            }))),
            sockets: false,
        }
    }

    #[cfg(test)]
    pub fn fixture_api_key(key: &str, endpoint: &str) -> Self {
        assert!(endpoint.starts_with("http://127.0.0.1:"));
        let mut api = Self::with_api_key(key.into()).unwrap();
        api.base_url = endpoint.into();
        api.sockets = false;
        api
    }

    #[cfg(test)]
    fn with_sockets(mut self) -> Self {
        self.sockets = true;
        self
    }

    fn catalog(&self) -> Result<Vec<backend::Model>, String> {
        let response = self.get("/models")?;
        let catalog = match &self.credential {
            Credential::Account(_) => metadata::parse(&response)?,
            Credential::ApiKey { .. } => metadata::parse_api(&response)?,
        };
        let models = catalog.models.clone();
        *self
            .metadata
            .lock()
            .map_err(|_| "OpenAI metadata is unavailable")? = Some(catalog);
        Ok(models)
    }

    fn ensure_metadata(&self) -> Result<(), String> {
        if self
            .metadata
            .lock()
            .map_err(|_| "OpenAI metadata is unavailable")?
            .is_none()
        {
            self.catalog()?;
        }
        Ok(())
    }

    fn known_limits(&self, model: &str) -> Option<backend::Limits> {
        self.metadata
            .lock()
            .ok()?
            .as_ref()?
            .limits
            .get(model)
            .copied()
    }

    fn supports_effort(&self, model: &str, effort: Effort) -> bool {
        self.metadata
            .lock()
            .ok()
            .and_then(|catalog| {
                catalog.as_ref().and_then(|catalog| {
                    catalog
                        .models
                        .iter()
                        .find(|entry| entry.id == model)
                        .map(|entry| entry.efforts.contains(&effort))
                })
            })
            .unwrap_or(false)
    }

    fn redactor(&self) -> Redactor {
        match &self.credential {
            Credential::Account(session) => session.redactor(),
            Credential::ApiKey { key, .. } => Redactor::new(key.clone()),
        }
    }
    fn account(&self) -> String {
        match &self.credential {
            Credential::Account(session) => session.identity(),
            Credential::ApiKey { scope, .. } => scope.clone(),
        }
    }
    fn route_id(&self) -> &'static str {
        match &self.credential {
            Credential::Account(_) => "openai",
            Credential::ApiKey { .. } => "openai-api",
        }
    }
}

impl OpenAi {
    pub fn with_api(api: Api, model: String) -> Result<Self, String> {
        backend::validate_model(&model)?;
        Ok(Self {
            api,
            model,
            effort: Effort::Default,
            conversation: None,
            delegation: true,
            channel: Mutex::default(),
        })
    }
}

impl Service for Api {
    fn validates_effort(&self) -> bool {
        false
    }
    fn id(&self) -> &'static str {
        self.route_id()
    }
    fn with_credential(&self, key: String) -> Result<Arc<dyn Service>, String> {
        match &self.credential {
            Credential::Account(_) => Err("OpenAI ChatGPT plan access requires Sign in with ChatGPT; an API key cannot replace this account session".into()),
            Credential::ApiKey { .. } => {
                let mut api = Self::with_api_key(key)?;
                api.base_url.clone_from(&self.base_url);
                api.sockets = self.sockets;
                api.cancellation = self.cancellation.clone();
                Ok(Arc::new(api))
            }
        }
    }
    fn with_cancellation(&self, cancellation: Cancellation) -> Arc<dyn Service> {
        let mut api = self.clone();
        api.cancellation = cancellation;
        Arc::new(api)
    }
    fn catalog(&self) -> Result<Vec<backend::Model>, String> {
        self.catalog()
    }
    fn models(&self, query: &str) -> Result<Vec<backend::Model>, String> {
        let query = query.to_ascii_lowercase();
        Ok(self
            .catalog()?
            .into_iter()
            .filter(|model| {
                model.id.to_ascii_lowercase().contains(&query)
                    || model.name.to_ascii_lowercase().contains(&query)
            })
            .collect())
    }
    fn check_credential(&self) -> Result<(), String> {
        self.catalog().map(|_| ())
    }
    fn redactor(&self) -> Redactor {
        self.redactor()
    }
    fn account(&self) -> Option<String> {
        Some(self.account())
    }
    fn create(&self, model: String) -> Result<Box<dyn Backend>, String> {
        Ok(Box::new(OpenAi::with_api(self.clone(), model)?))
    }
}

impl Backend for OpenAi {
    fn id(&self) -> &'static str {
        self.api.route_id()
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
    fn maintenance_effort(&self, requested: Effort) -> Effort {
        if self.api.supports_effort(&self.model, requested) {
            requested
        } else {
            Effort::Default
        }
    }
    fn set_delegation(&mut self, enabled: bool) {
        self.delegation = enabled;
    }
    fn inputs(&self) -> Inputs {
        Inputs::default()
    }
    fn limits(&mut self) -> Result<Option<backend::Limits>, backend::Failure> {
        self.api.ensure_metadata().map_err(backend::Failure::from)?;
        Ok(self.api.known_limits(&self.model))
    }
    fn known_limits(&self) -> Option<backend::Limits> {
        self.api.known_limits(&self.model)
    }
    fn sample(
        &self,
        messages: &[Value],
        with_tools: bool,
        output_tokens: Option<usize>,
        pool: Option<&Pool>,
        emit: &mut dyn FnMut(backend::Update) -> Result<(), String>,
    ) -> Result<backend::Completion, backend::Failure> {
        self.sample_with_effort(self.effort, messages, with_tools, output_tokens, pool, emit)
    }
    fn sample_with_effort(
        &self,
        effort: Effort,
        messages: &[Value],
        with_tools: bool,
        output_tokens: Option<usize>,
        pool: Option<&Pool>,
        emit: &mut dyn FnMut(backend::Update) -> Result<(), String>,
    ) -> Result<backend::Completion, backend::Failure> {
        let origin = Origin::with_account(self.id(), self.model(), Some(self.api.account()));
        let projected: Vec<_> = messages
            .iter()
            .map(|message| conversation::project(message, &origin))
            .collect();
        let mut request = wire::request_with_catalog(
            &self.model,
            effort,
            &projected,
            with_tools.then(|| crate::tools::definitions_for(self.delegation)),
            pool,
        )
        .map_err(backend::Failure::from)?;
        if let (Credential::ApiKey { .. }, Some(tokens), Value::Object(fields)) =
            (&self.api.credential, output_tokens, &mut request)
        {
            fields.insert("max_output_tokens".into(), Value::number(tokens));
        }
        // A stable conversation key supports provider prompt-cache affinity.
        if let (Some(id), Value::Object(fields)) = (&self.conversation, &mut request) {
            fields.insert("prompt_cache_key".into(), Value::string(id));
        }
        let body = request.encode();
        emit(backend::Update::Waiting {
            request_bytes: body.len(),
        })
        .map_err(backend::Failure::from)?;
        let mut stream = stream::Stream::default();
        let answered = socket::exchange(
            &self.channel,
            &self.api,
            self.conversation.as_deref(),
            request,
            &mut stream,
            emit,
        )
        .map_err(|mut failure| {
            transport::sanitize(&mut failure, &self.api.redactor());
            failure
        })?;
        let response = if answered {
            None
        } else {
            emit(backend::Update::Transport("http")).map_err(backend::Failure::from)?;
            Some(self.api.post_stream(
                "/responses",
                &body,
                self.conversation.as_deref(),
                &mut |chunk| stream.feed(chunk, emit),
            )?)
        };
        let mut completion = stream.finish().map_err(|mut failure| {
            if let Some(response) = response {
                failure.message = format!("{} (HTTP {})", failure.message, response.status);
                failure.retry_after = response.retry_after;
                if let Some(id) = response.request_id {
                    failure.message.push_str(&format!("; request id: {id}"));
                }
            }
            transport::sanitize(&mut failure, &self.api.redactor());
            failure
        })?;
        conversation::stamp(&mut completion.message, &origin);
        Ok(completion)
    }
    fn redactor(&self) -> Redactor {
        self.api.redactor()
    }
    fn set_cancellation(&mut self, cancellation: Cancellation) {
        self.api.cancellation = cancellation;
    }
    fn set_conversation(&mut self, id: Option<String>) {
        // ChatGPT also sends this as an HTTP header, so only plain tokens qualify.
        let next = id.filter(|id| {
            !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        });
        if self.conversation != next {
            self.channel
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .reset();
        }
        self.conversation = next;
    }
    fn services(&self) -> backend::Services {
        backend::Services::from_service(self.api.clone())
    }
    fn account(&self) -> Option<String> {
        Some(self.api.account())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
