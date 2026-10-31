//! OpenRouter's implementation of the model and account boundaries.

use super::{Api, OpenRouter};
use crate::attachments::{Inputs, Pool};
use crate::backend::{self, Backend, Service};
use crate::cancel::Cancellation;
use crate::conversation::{self, Origin};
use crate::effort::Effort;
use crate::json::Value;
use crate::redact::Redactor;
use std::any::Any;
use std::sync::Arc;

impl Backend for OpenRouter {
    fn id(&self) -> &'static str {
        "openrouter"
    }
    fn account(&self) -> Option<String> {
        Some(self.api.account.clone())
    }
    fn model(&self) -> &str {
        self.model()
    }
    fn effort(&self) -> Effort {
        self.effort()
    }
    fn set_effort(&mut self, effort: Effort) {
        self.set_effort(effort);
    }
    fn maintenance_effort(&self, requested: Effort) -> Effort {
        let supported = self
            .efforts
            .as_ref()
            .is_some_and(|efforts| efforts.contains(&requested));
        let pinned_without_reasoning = self
            .router
            .pinned(&self.model)
            .is_some_and(|endpoint| endpoint.supports("reasoning") == Some(false));
        if supported && !pinned_without_reasoning {
            requested
        } else {
            Effort::Default
        }
    }
    fn set_delegation(&mut self, enabled: bool) {
        self.delegation = enabled;
    }
    fn inputs(&self) -> Inputs {
        self.inputs()
    }
    fn limits(&mut self) -> Result<Option<backend::Limits>, backend::Failure> {
        self.limits()
    }
    fn known_limits(&self) -> Option<backend::Limits> {
        self.known_limits()
    }
    fn sample(
        &self,
        messages: &[Value],
        with_tools: bool,
        output_tokens: Option<usize>,
        pool: Option<&Pool>,
        emit: &mut dyn FnMut(backend::Update) -> Result<(), String>,
    ) -> Result<backend::Completion, backend::Failure> {
        self.sample_with_effort(
            self.effort(),
            messages,
            with_tools,
            output_tokens,
            pool,
            emit,
        )
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
        let request_state = messages
            .last()
            .is_some_and(|message| message.get("jecode_request_state") == Some(&Value::Bool(true)));
        let origin = Origin::with_account(self.id(), self.model(), self.account());
        let projected: Vec<_> = messages
            .iter()
            .map(|message| {
                let mut projected = conversation::project(message, &origin);
                if let Value::Object(fields) = &mut projected {
                    fields.remove("jecode_request_state");
                }
                projected
            })
            .collect();
        let projected = super::calls::request(projected).map_err(backend::Failure::from)?;
        let request = super::attachments::materialize(projected, pool, self.inputs())
            .map_err(backend::Failure::from)?;
        let mut forward = |update| emit(update);
        let mut completion = self.sample_owned(
            request,
            effort,
            with_tools,
            request_state,
            output_tokens,
            &mut forward,
        )?;
        super::calls::response(&mut completion.message, &completion.calls);
        if let Err(error) = super::annotations::store(&mut completion.message, pool) {
            if let Value::Object(fields) = &mut completion.message {
                fields.remove("annotations");
            }
            completion.postprocess_error =
                Some(format!("Could not store PDF annotations: {error}"));
        }
        conversation::stamp(&mut completion.message, &origin);
        Ok(completion)
    }
    fn redactor(&self) -> Redactor {
        self.redactor()
    }
    fn set_cancellation(&mut self, cancellation: Cancellation) {
        self.set_cancellation(cancellation);
    }
    fn set_conversation(&mut self, id: Option<String>) {
        self.set_conversation(id);
    }
    fn services(&self) -> backend::Services {
        backend::Services::from_service(self.api())
    }
    fn prepare(&mut self) -> Result<(), String> {
        self.refresh_endpoints();
        self.router().validate_pin(
            self.model(),
            self.effort().reasons(),
            std::time::Instant::now(),
        )
    }
    fn prepare_turn(&mut self) -> Result<Option<backend::Limits>, backend::Failure> {
        let parallel = self
            .router
            .needs_refresh(&self.model, std::time::Instant::now())
            && self.limits.is_none();
        let fetched = if parallel {
            let api = self.api.clone();
            let model = self.model.clone();
            Some(std::thread::scope(|scope| {
                let endpoints = scope.spawn(|| api.endpoints(&model));
                let limits = self.model_limits();
                match endpoints.join().expect("endpoint metadata worker panicked") {
                    Ok(endpoints) => self.router.offer(endpoints),
                    Err(_) => self.router.failed(&self.model, std::time::Instant::now()),
                }
                limits
            }))
        } else {
            self.refresh_endpoints();
            None
        };
        if self.api.cancellation.requested() {
            return Err(backend::Failure::from("Operation cancelled"));
        }
        self.router
            .validate_pin(&self.model, self.reasoning(), std::time::Instant::now())
            .map_err(backend::Failure::from)?;
        let limits = match fetched {
            Some(limits) => limits?,
            None => self.model_limits()?,
        };
        Ok(self.router.limits(&self.model, limits))
    }
    fn routing(&self) -> backend::Routing {
        self.routing().clone()
    }
    fn set_routing(&mut self, routing: backend::Routing) {
        self.set_routing(routing);
    }
    fn offer_endpoints(&mut self, endpoints: backend::Endpoints) {
        self.offer_endpoints(endpoints);
    }
    fn cached_endpoints(&self) -> Option<backend::Endpoints> {
        self.router()
            .endpoints(self.model())
            .filter(|endpoints| endpoints.fresh(std::time::Instant::now()))
            .cloned()
    }
    fn clone_routing_from(&mut self, previous: &dyn Backend) {
        if let Some(previous) = previous.as_any().downcast_ref::<OpenRouter>() {
            self.set_router(previous.router().clone());
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    #[cfg(test)]
    fn fixture_limits(&mut self, context: usize, output: Option<usize>) {
        self.fixture_limits(context, output);
    }
    #[cfg(test)]
    fn fixture_unknown_limits(&mut self) {
        self.fixture_unknown_limits();
    }
}

impl Service for Api {
    fn id(&self) -> &'static str {
        "openrouter"
    }
    fn account(&self) -> Option<String> {
        Some(self.account.clone())
    }
    fn with_credential(&self, key: String) -> Result<Arc<dyn Service>, String> {
        Ok(Arc::new(self.clone().with_key(key)?))
    }
    fn with_cancellation(&self, cancellation: Cancellation) -> Arc<dyn Service> {
        Arc::new(self.clone().with_cancellation(cancellation))
    }
    fn catalog(&self) -> Result<Vec<backend::Model>, String> {
        self.catalog()
    }
    fn models(&self, query: &str) -> Result<Vec<backend::Model>, String> {
        self.models(query)
    }
    fn check_credential(&self) -> Result<(), String> {
        self.check_key()
    }
    fn redactor(&self) -> Redactor {
        self.redactor()
    }
    fn endpoints(&self, model: &str) -> Result<Option<backend::Endpoints>, String> {
        self.endpoints(model).map(Some)
    }
    fn routing_entries(
        &self,
        endpoints: Option<&backend::Endpoints>,
        current: &backend::Routing,
        reasoning: bool,
    ) -> Option<Vec<backend::Entry>> {
        Some(super::routing::entries(endpoints, current, reasoning))
    }
    fn create(&self, model: String) -> Result<Box<dyn Backend>, String> {
        Ok(Box::new(OpenRouter::with_api(self.clone(), model)?))
    }
}
