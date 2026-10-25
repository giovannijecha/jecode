//! Keeps local conversation tools available before a provider can run inference.
use super::*;

#[derive(Clone)]
struct Unconfigured {
    id: &'static str,
    model: String,
    effort: Effort,
    services: Option<Services>,
    message: String,
}

pub fn client(
    id: &str,
    model: String,
    services: Option<Services>,
    message: String,
) -> Box<dyn Backend> {
    Box::new(Unconfigured {
        id: available()
            .iter()
            .find(|info| info.id == id)
            .map_or("openrouter", |info| info.id),
        model,
        effort: Effort::Default,
        services,
        message,
    })
}

impl Backend for Unconfigured {
    fn needs_configuration(&self) -> bool {
        true
    }
    fn id(&self) -> &'static str {
        self.id
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
        _messages: &[Value],
        _with_tools: bool,
        _output_tokens: Option<usize>,
        _pool: Option<&Pool>,
        _emit: &mut dyn FnMut(Update) -> Result<(), String>,
    ) -> Result<Completion, Failure> {
        Err(self.message.clone().into())
    }
    fn prepare(&mut self) -> Result<(), String> {
        Err(self.message.clone())
    }
    fn redactor(&self) -> Redactor {
        self.services
            .as_ref()
            .map_or_else(|| Redactor::new(String::new()), Services::redactor)
    }
    fn set_cancellation(&mut self, _cancellation: Cancellation) {}
    fn set_conversation(&mut self, _id: Option<String>) {}
    fn services(&self) -> Services {
        self.services
            .clone()
            .unwrap_or_else(|| Services::from_service(self.clone()))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Service for Unconfigured {
    fn id(&self) -> &'static str {
        self.id
    }
    fn with_credential(&self, key: String) -> Result<Arc<dyn Service>, String> {
        Ok(Services::for_backend(self.id, key)?.inner)
    }
    fn with_cancellation(&self, _cancellation: Cancellation) -> Arc<dyn Service> {
        Arc::new(self.clone())
    }
    fn catalog(&self) -> Result<Vec<Model>, String> {
        Err(self.message.clone())
    }
    fn models(&self, _query: &str) -> Result<Vec<Model>, String> {
        Err(self.message.clone())
    }
    fn check_credential(&self) -> Result<(), String> {
        Err(self.message.clone())
    }
    fn redactor(&self) -> Redactor {
        Backend::redactor(self)
    }
    fn create(&self, _model: String) -> Result<Box<dyn Backend>, String> {
        Err(self.message.clone())
    }
}
