//! ChatGPT plan inference over OpenAI's public Responses API.

#[cfg(test)]
mod agent_tests;
#[cfg(test)]
mod live;
mod metadata;
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

#[derive(Clone)]
pub struct Api {
    session: Session,
    cancellation: Cancellation,
    base_url: String,
    metadata: Arc<Mutex<Option<metadata::Catalog>>>,
}

pub struct OpenAi {
    api: Api,
    model: String,
    effort: Effort,
    /// The saved session, sent as the prompt cache key and `session-id`.
    conversation: Option<String>,
    delegation: bool,
}

impl Api {
    pub fn new(session: Session) -> Result<Self, String> {
        Ok(Self {
            session,
            cancellation: Cancellation::default(),
            base_url: "https://api.openai.com/v1".into(),
            metadata: Arc::new(Mutex::new(None)),
        })
    }

    #[cfg(test)]
    pub fn fixture(session: Session, endpoint: &str) -> Self {
        assert!(endpoint.starts_with("http://127.0.0.1:"));
        Self {
            session,
            cancellation: Cancellation::default(),
            base_url: endpoint.into(),
            metadata: Arc::new(Mutex::new(Some(metadata::Catalog {
                models: vec![],
                limits: Default::default(),
            }))),
        }
    }

    fn catalog(&self) -> Result<Vec<backend::Model>, String> {
        let response = self.get("/models")?;
        let catalog = metadata::parse(&response)?;
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

    fn redactor(&self) -> Redactor {
        self.session.redactor()
    }
    fn account(&self) -> String {
        self.session.identity()
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
        })
    }
}

impl Service for Api {
    fn validates_effort(&self) -> bool {
        false
    }
    fn id(&self) -> &'static str {
        "openai"
    }
    fn with_credential(&self, _key: String) -> Result<Arc<dyn Service>, String> {
        Err("OpenAI ChatGPT plan access requires Sign in with ChatGPT; an API key cannot replace this account session".into())
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
        "openai"
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
        _output_tokens: Option<usize>,
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
            self.effort,
            &projected,
            with_tools.then(|| crate::tools::definitions_for(self.delegation)),
            pool,
        )
        .map_err(backend::Failure::from)?;
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
        let response = self.api.post_stream(
            "/responses",
            &body,
            self.conversation.as_deref(),
            &mut |chunk| stream.feed(chunk, emit),
        )?;
        let mut completion = stream.finish().map_err(|mut failure| {
            failure.message = format!("{} (HTTP {})", failure.message, response.status);
            failure.retry_after = response.retry_after;
            if let Some(id) = response.request_id {
                failure.message.push_str(&format!("; request id: {id}"));
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
        // The id also travels as an HTTP header, so only plain tokens qualify.
        self.conversation = id.filter(|id| {
            !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        });
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
