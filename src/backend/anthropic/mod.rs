//! Claude subscription inference over the native Messages API.

#[cfg(test)]
mod cache_tests;
#[cfg(test)]
mod efficiency_live;
#[cfg(test)]
mod live;
mod metadata;
mod stream;
#[cfg(test)]
mod tests;
mod transport;
#[cfg(test)]
mod usage_tests;
mod wire;

use crate::attachments::{Inputs, Pool};
use crate::auth::anthropic::Session;
use crate::backend::{self, Backend, Service};
use crate::cancel::Cancellation;
use crate::conversation::{self, Origin};
use crate::effort::Effort;
use crate::json::Value;
use crate::redact::Redactor;
use std::any::Any;
use std::sync::{Arc, Mutex};

pub const CREDENTIAL_ENV: &[&str] = &["JECODE_ANTHROPIC_ACCESS_TOKEN"];

#[derive(Clone)]
pub struct Api {
    session: Session,
    cancellation: Cancellation,
    base_url: String,
    metadata: Arc<Mutex<Option<metadata::Catalog>>>,
}

pub struct Anthropic {
    api: Api,
    model: String,
    effort: Effort,
    delegation: bool,
}

impl Api {
    pub fn new(session: Session) -> Result<Self, String> {
        Ok(Self {
            session,
            cancellation: Cancellation::default(),
            base_url: "https://api.anthropic.com".into(),
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
            metadata: Arc::new(Mutex::new(None)),
        }
    }

    fn catalog(&self) -> Result<Vec<backend::Model>, String> {
        let mut result = metadata::Catalog::default();
        let mut next = Some("/v1/models?limit=100".to_owned());
        while let Some(path) = next.take() {
            let page = self.get(&path)?;
            let parsed = metadata::parse(&page)?;
            result.models.extend(parsed.models);
            result.limits.extend(parsed.limits);
            result.thinking_off.extend(parsed.thinking_off);
            result.thinking_adaptive.extend(parsed.thinking_adaptive);
            next = match page.get("has_more") {
                Some(Value::Bool(true)) => page
                    .get("last_id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(|id| format!("/v1/models?limit=100&after_id={}", percent(id))),
                _ => None,
            };
            if result.models.len() > 1000 {
                return Err("Anthropic model catalog is too large".into());
            }
        }
        let models = result.models.clone();
        *self
            .metadata
            .lock()
            .map_err(|_| "Anthropic metadata unavailable")? = Some(result);
        Ok(models)
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

    fn thinking_off(&self, model: &str) -> Option<&'static str> {
        self.metadata
            .lock()
            .ok()?
            .as_ref()?
            .thinking_off
            .get(model)
            .copied()
    }

    fn thinking_adaptive(&self, model: &str) -> bool {
        self.metadata
            .lock()
            .ok()
            .and_then(|catalog| {
                catalog
                    .as_ref()
                    .map(|catalog| catalog.thinking_adaptive.contains(model))
            })
            .unwrap_or(false)
    }

    fn supports_effort(&self, model: &str, effort: Effort) -> Option<bool> {
        self.metadata.lock().ok()?.as_ref().and_then(|catalog| {
            catalog
                .models
                .iter()
                .find(|entry| entry.id == model)
                .map(|entry| entry.efforts.contains(&effort))
        })
    }

    fn ensure_metadata(&self) -> Result<(), String> {
        if self
            .metadata
            .lock()
            .map_err(|_| "Anthropic metadata unavailable")?
            .is_none()
        {
            self.catalog()?;
        }
        Ok(())
    }

    fn redactor(&self) -> Redactor {
        self.session.redactor()
    }
}

fn percent(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

impl Service for Api {
    fn id(&self) -> &'static str {
        "anthropic"
    }
    fn account(&self) -> Option<String> {
        Some(self.session.identity())
    }
    fn with_credential(&self, _key: String) -> Result<Arc<dyn Service>, String> {
        Err("Claude subscription access requires Sign in with Claude; an API key cannot replace this account session".into())
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
    fn create(&self, model: String) -> Result<Box<dyn Backend>, String> {
        backend::validate_model(&model)?;
        Ok(Box::new(Anthropic {
            api: self.clone(),
            model,
            effort: Effort::Default,
            delegation: true,
        }))
    }
}

impl Backend for Anthropic {
    fn id(&self) -> &'static str {
        "anthropic"
    }
    fn account(&self) -> Option<String> {
        Some(self.api.session.identity())
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
        if self.api.supports_effort(&self.model, requested) == Some(true) {
            requested
        } else {
            Effort::Default
        }
    }
    fn set_delegation(&mut self, enabled: bool) {
        self.delegation = enabled;
    }
    fn inputs(&self) -> Inputs {
        Inputs {
            image: Some(true),
            file: Some(true),
        }
    }
    fn limits(&mut self) -> Result<Option<backend::Limits>, backend::Failure> {
        self.api.ensure_metadata().map_err(backend::Failure::from)?;
        Ok(self.known_limits())
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
        let origin = Origin::with_account(self.id(), self.model(), self.account());
        let projected: Vec<_> = messages
            .iter()
            .map(|message| conversation::project(message, &origin))
            .collect();
        let thinking_mode = if effort == Effort::None {
            self.api.thinking_off(&self.model)
        } else if self.api.thinking_adaptive(&self.model) {
            Some("adaptive")
        } else {
            None
        };
        let request = wire::request(
            &self.model,
            effort,
            &projected,
            with_tools.then(|| crate::tools::definitions_for(self.delegation)),
            output_tokens.or_else(|| self.known_limits().and_then(|v| v.output)),
            thinking_mode,
            pool,
        )
        .map_err(backend::Failure::from)?;
        let body = request.encode();
        emit(backend::Update::Waiting {
            request_bytes: body.len(),
        })
        .map_err(backend::Failure::from)?;
        let mut stream = stream::Stream::default();
        let response = self
            .api
            .post_stream(&body, &mut |bytes| stream.feed(bytes, emit))?;
        let mut completion = stream.finish().map_err(|mut failure| {
            failure
                .message
                .push_str(&format!(" (HTTP {})", response.status));
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
    fn set_conversation(&mut self, _id: Option<String>) {}
    fn services(&self) -> backend::Services {
        backend::Services::from_service(self.api.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
