//! What OpenRouter reported about served requests: the provider and the
//! charged cost. Nothing here is inferred from the routing preference.

use crate::backend::Charges;
use crate::json::Value;

pub(super) fn usage(value: Option<&Value>) -> Option<crate::backend::Usage> {
    let value = value?;
    let input_tokens = value.get("prompt_tokens").and_then(Value::as_usize);
    let output_tokens = value.get("completion_tokens").and_then(Value::as_usize);
    let cached_tokens = value
        .get("prompt_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_usize);
    (input_tokens.is_some() || output_tokens.is_some() || cached_tokens.is_some()).then_some(
        crate::backend::Usage {
            input_tokens,
            output_tokens,
            cached_tokens,
        },
    )
}

impl Charges {
    pub(super) fn reported(provider: Option<&str>, usage: Option<&Value>) -> Option<Self> {
        if provider.is_none() && usage.is_none() {
            return None;
        }
        let mut charges = Self::default();
        charges.record(provider, usage);
        Some(charges)
    }

    fn record(&mut self, provider: Option<&str>, usage: Option<&Value>) {
        self.requests += 1;
        if let Some(provider) = provider.map(str::trim).filter(|name| !name.is_empty())
            && !self
                .providers
                .iter()
                .any(|known| known.eq_ignore_ascii_case(provider))
        {
            self.providers.push(provider.into());
        }
        let Some(usage) = usage else {
            return;
        };
        if let Some(cost) = number(usage.get("cost")).filter(|cost| *cost >= 0.0) {
            self.priced += 1;
            self.cost += cost;
        }
        self.cached += usage
            .get("prompt_tokens_details")
            .and_then(|details| details.get("cached_tokens"))
            .and_then(Value::as_usize)
            .unwrap_or(0);
        if usage.get("is_byok") == Some(&Value::Bool(true)) {
            self.byok = true;
            self.upstream += number(
                usage
                    .get("cost_details")
                    .and_then(|details| details.get("upstream_inference_cost")),
            )
            .filter(|cost| *cost >= 0.0)
            .unwrap_or(0.0);
        }
    }
}

/// The selected routing candidate, with the older response field as a fallback.
pub(super) fn provider(value: &Value) -> Option<String> {
    value
        .get("openrouter_metadata")
        .and_then(|metadata| metadata.get("endpoints"))
        .and_then(|endpoints| endpoints.get("available"))
        .and_then(Value::as_array)
        .and_then(|endpoints| {
            endpoints.iter().find_map(|endpoint| {
                (endpoint.get("selected") == Some(&Value::Bool(true)))
                    .then(|| endpoint.get("provider").and_then(Value::as_str))
                    .flatten()
                    .filter(|provider| !provider.trim().is_empty())
            })
        })
        .or_else(|| value.get("provider").and_then(Value::as_str))
        .map(str::to_owned)
}

fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.parse::<f64>().ok().filter(|value| value.is_finite()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(text: &str) -> Value {
        crate::json::parse(text).unwrap()
    }

    #[test]
    fn reported_providers_and_charges_are_summed_without_estimates() {
        let mut charges = Charges::default();
        assert_eq!(charges.describe(), None);
        charges.record(
            Some("Baidu"),
            Some(&usage(
                r#"{"cost":0.0021,"prompt_tokens_details":{"cached_tokens":12000}}"#,
            )),
        );
        charges.record(Some(" baidu "), Some(&usage(r#"{"cost":0.001}"#)));
        assert_eq!(
            charges.describe().unwrap(),
            "Served by Baidu · $0.0031 charged · 12K cached"
        );
        // Missing or invalid costs stay unknown and are not counted as free.
        charges.record(Some("DeepInfra"), Some(&usage(r#"{"cost":"0.5"}"#)));
        charges.record(None, None);
        charges.record(Some("Novita"), Some(&usage(r#"{"cost":-1}"#)));
        charges.record(Some("Groq"), Some(&usage("{}")));
        assert_eq!(
            charges.describe().unwrap(),
            "Served by Baidu, DeepInfra +2 · $0.0031 charged for 2 of 6 requests · 12K cached"
        );
    }

    #[test]
    fn own_key_requests_report_the_upstream_charge_separately() {
        let mut charges = Charges::default();
        charges.record(
            Some("OpenAI"),
            Some(&usage(
                r#"{"cost":0,"is_byok":true,"cost_details":{"upstream_inference_cost":0.012}}"#,
            )),
        );
        assert_eq!(
            charges.describe().unwrap(),
            "Served by OpenAI · $0 charged · BYOK $0.012 upstream"
        );
        let mut unknown = Charges::default();
        unknown.record(None, Some(&usage(r#"{"is_byok":true}"#)));
        assert_eq!(unknown.describe().unwrap(), "BYOK");
    }
}
