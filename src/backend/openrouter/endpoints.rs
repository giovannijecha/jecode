//! Endpoint metadata from `GET /models/{author}/{slug}/endpoints`.

use super::Api;
pub use crate::backend::{Endpoint, Endpoints};
use crate::json::Value;
use std::time::Instant;

impl Endpoints {
    pub fn fresh(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.fetched) < super::routing::policy::FRESH
    }
}

impl Api {
    pub fn endpoints(&self, model: &str) -> Result<Endpoints, String> {
        let (author, slug) = model
            .split_once('/')
            .ok_or("Endpoint listing needs an author/slug model ID")?;
        let path = format!(
            "/models/{}/{}/endpoints",
            super::models::percent_encode(author),
            super::models::percent_encode(slug)
        );
        parse(model, &self.request("GET", &path, None)?, Instant::now())
    }
}

pub(super) fn parse(model: &str, value: &Value, fetched: Instant) -> Result<Endpoints, String> {
    let list = value
        .get("data")
        .and_then(|data| data.get("endpoints"))
        .and_then(Value::as_array)
        .ok_or("OpenRouter returned an invalid endpoint listing")?;
    let mut endpoints: Vec<Endpoint> = Vec::new();
    for value in list {
        let Some(tag) = value
            .get("tag")
            .and_then(Value::as_str)
            .filter(|tag| super::routing::valid_tag(tag))
        else {
            continue;
        };
        if endpoints.iter().any(|endpoint| endpoint.tag == tag) {
            continue;
        }
        let pricing = value.get("pricing");
        let price = |key| pricing.and_then(|pricing| per_million(pricing.get(key)));
        let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
        let workload = value
            .get("perf_last_30m_by_workload")
            .and_then(|workloads| workloads.get("text_generation"));
        let median = |stats: Option<&Value>| {
            number(stats.and_then(|stats| stats.get("p50"))).filter(|value| *value > 0.0)
        };
        endpoints.push(Endpoint {
            tag: tag.into(),
            provider: text(value.get("provider_name")),
            prompt: price("prompt"),
            completion: price("completion"),
            cache_read: price("input_cache_read"),
            cache_write: price("input_cache_write"),
            discount: number(pricing.and_then(|pricing| pricing.get("discount")))
                .filter(|discount| (0.0..1.0).contains(discount)),
            context: value
                .get("context_length")
                .and_then(Value::as_usize)
                .filter(|tokens| *tokens > 0),
            output: value
                .get("max_completion_tokens")
                .and_then(Value::as_usize)
                .filter(|tokens| *tokens > 0),
            quantization: text(value.get("quantization"))
                .filter(|quantization| quantization != "unknown"),
            tier: text(value.get("service_tier"))
                .filter(|tier| !matches!(tier.as_str(), "default" | "standard")),
            status: number(value.get("status"))
                .filter(|status| status.fract() == 0.0)
                .map(|status| status as i64),
            uptime: number(value.get("uptime_last_30m"))
                .filter(|uptime| (0.0..=100.0).contains(uptime)),
            // The legacy latency field's unit is inconsistent in the published
            // examples, so only the workload statistics, in milliseconds, count.
            latency: median(workload.and_then(|workload| workload.get("latency"))),
            throughput: median(workload.and_then(|workload| workload.get("throughput")))
                .or_else(|| median(value.get("throughput_last_30m"))),
            requests: workload
                .and_then(|workload| workload.get("request_count"))
                .and_then(Value::as_usize)
                .map(|count| count as u64),
            parameters: value
                .get("supported_parameters")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                }),
        });
    }
    Ok(Endpoints {
        model: model.into(),
        list: endpoints,
        fetched,
    })
}

fn number(value: Option<&Value>) -> Option<f64> {
    let value = match value? {
        Value::Number(number) | Value::String(number) => number.parse::<f64>().ok()?,
        _ => return None,
    };
    value.is_finite().then_some(value)
}

/// A per-token price string as USD per million tokens; negative values,
/// which OpenRouter uses for variable pricing, are unknown.
fn per_million(value: Option<&Value>) -> Option<f64> {
    number(value)
        .map(|price| price * 1_000_000.0)
        .filter(|price| price.is_finite() && *price >= 0.0)
}

#[cfg(test)]
mod tests;
