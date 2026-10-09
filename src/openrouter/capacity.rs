use super::{Failure, OpenRouter};
use crate::json::Value;

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

#[cfg(test)]
mod tests;

impl OpenRouter {
    pub fn limits(&mut self) -> Result<Option<Limits>, Failure> {
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
        self.summary_json = model
            .and_then(|model| model.get("supported_parameters"))
            .and_then(Value::as_array)
            .is_some_and(|parameters| {
                parameters
                    .iter()
                    .any(|value| value.as_str() == Some("response_format"))
            });
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

    #[cfg(test)]
    pub fn fixture_limits(&mut self, context: usize, output: Option<usize>) {
        self.limits = Some(Some(Limits { context, output }));
    }
}
