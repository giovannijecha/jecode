use super::{Failure, OpenRouter};
pub use crate::backend::Limits;
use crate::json::Value;

#[cfg(test)]
mod tests;

impl OpenRouter {
    pub fn limits(&mut self) -> Result<Option<Limits>, Failure> {
        let limits = self.model_limits()?;
        Ok(self.router.limits(&self.model, limits))
    }

    /// Model catalog capacity before endpoint-specific narrowing.
    pub(super) fn model_limits(&mut self) -> Result<Option<Limits>, Failure> {
        if let Some(limits) = self.limits {
            return Ok(limits);
        }
        let value = self.api.send(
            "GET",
            &format!(
                "/models?supported_parameters=tools&q={}&limit=10",
                super::models::percent_encode(&self.model)
            ),
            None,
        )?;
        let data = value
            .get("data")
            .and_then(Value::as_array)
            .ok_or("OpenRouter returned an invalid model catalog")?;
        let model = data
            .iter()
            .find(|model| model.get("id").and_then(Value::as_str) == Some(&self.model));
        self.efforts = model.map(super::models::efforts);
        self.inputs = model
            .and_then(|model| model.get("architecture"))
            .and_then(|architecture| architecture.get("input_modalities"))
            .and_then(Value::as_array)
            .map(|inputs| {
                inputs
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            });
        let limits = model.and_then(|model| {
            let context = model
                .get("context_length")
                .and_then(Value::as_usize)
                .filter(|tokens| *tokens > 0)?;
            let output = model
                .get("top_provider")
                .and_then(|provider| provider.get("max_completion_tokens"))
                .and_then(Value::as_usize)
                .filter(|tokens| *tokens > 0);
            Some(Limits { context, output })
        });
        self.limits = Some(limits);
        Ok(limits)
    }

    /// Limits already read for this model, narrowed to a pinned endpoint,
    /// without contacting the catalog.
    pub fn known_limits(&self) -> Option<Limits> {
        self.router.limits(&self.model, self.limits.flatten())
    }

    #[cfg(test)]
    pub fn fixture_limits(&mut self, context: usize, output: Option<usize>) {
        self.limits = Some(Some(Limits { context, output }));
    }

    /// A catalog entry without a context length.
    #[cfg(test)]
    pub fn fixture_unknown_limits(&mut self) {
        self.limits = Some(None);
    }
}
