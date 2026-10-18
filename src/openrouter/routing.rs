//! Which of a model's OpenRouter endpoints serve a conversation's requests.
//!
//! Auto leaves endpoint choice to OpenRouter's native routing, which weighs
//! price, latency, throughput and uptime, keeps fallbacks and, for tool
//! requests, Auto Exacto. It never sorts by price, which would bypass Auto
//! Exacto. It adds a stable `session_id` to help reuse a warm endpoint, and,
//! only when fresh measurements exist, a responsiveness guard.
//! A specific endpoint is an explicit pin without fallbacks.

mod options;

pub use options::{Entry, entries};

use super::Limits;
use super::endpoints::{Endpoint, Endpoints};
use crate::json::Value;
use std::time::Instant;

/// Auto's thresholds, kept together. All are provisional: they follow the
/// routing policy's intent and are not calibrated against measured traffic.
pub mod policy {
    use std::time::Duration;

    /// Comparable measured endpoints needed before Auto states any preference.
    pub const MIN_MEASURED: usize = 3;
    /// Requests behind an endpoint's statistics below which they are too sparse to compare.
    pub const MIN_REQUESTS: u64 = 10;
    /// Endpoints below this share of the median throughput are tried after the others.
    pub const THROUGHPUT_SHARE: f64 = 0.5;
    /// Endpoints above this multiple of the median first-token latency are tried after the others.
    pub const LATENCY_MULTIPLE: f64 = 2.0;
    /// How long fetched endpoint metadata guides requests.
    pub const FRESH: Duration = Duration::from_secs(10 * 60);
    /// How long a failed metadata fetch waits before the next attempt.
    pub const RETRY: Duration = Duration::from_secs(60);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Routing {
    /// Auto — Best balance.
    Auto,
    /// One endpoint tag, without fallbacks.
    Endpoint(String),
    /// No routing fields, as conversations saved before routing existed.
    Default,
}

impl Routing {
    /// A saved routing; None for unknown modes, which callers treat by context.
    pub fn parse(value: &Value) -> Option<Self> {
        match value.get("mode")?.as_str()? {
            "auto" => Some(Self::Auto),
            "default" => Some(Self::Default),
            "endpoint" => value
                .get("endpoint")
                .and_then(Value::as_str)
                .filter(|tag| valid_tag(tag))
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
            Self::Default => "OpenRouter default".into(),
        }
    }
}

pub fn valid_tag(tag: &str) -> bool {
    !tag.is_empty() && tag.len() <= 128 && tag.bytes().all(|byte| byte.is_ascii_graphic())
}

/// Auto's responsiveness preferences, relative to typical measured endpoints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guard {
    /// Preferred minimum p50 output throughput, tokens per second.
    pub throughput: f64,
    /// Preferred maximum p50 time to first token, seconds.
    pub latency: f64,
    pub measured: usize,
}

/// Whether an endpoint can serve this conversation's tool requests; unknown
/// parameter support is not held against it.
pub fn compatible(endpoint: &Endpoint, reasoning: bool) -> bool {
    endpoint.supports("tools") != Some(false)
        && (!reasoning || endpoint.supports("reasoning") != Some(false))
}

/// Preferences that move markedly slow outliers behind typical endpoints.
/// Medians, not the fastest endpoint, set the cutoffs. Unmeasured endpoints
/// do not influence those cutoffs; native routing still decides the provider.
pub fn guard(endpoints: &Endpoints, reasoning: bool) -> Option<Guard> {
    let measured: Vec<_> = endpoints
        .list
        .iter()
        .filter(|endpoint| {
            endpoint.tier.is_none()
                && endpoint.status == Some(0)
                && compatible(endpoint, reasoning)
                && endpoint
                    .requests
                    .is_none_or(|requests| requests >= policy::MIN_REQUESTS)
        })
        .filter_map(|endpoint| Some((endpoint.throughput?, endpoint.latency?)))
        .collect();
    if measured.len() < policy::MIN_MEASURED {
        return None;
    }
    let throughput = median(measured.iter().map(|(throughput, _)| *throughput));
    let latency = median(measured.iter().map(|(_, latency)| *latency));
    Some(Guard {
        throughput: round(throughput * policy::THROUGHPUT_SHARE, 10.0).max(0.1),
        latency: round(latency * policy::LATENCY_MULTIPLE / 1000.0, 100.0).max(0.01),
        measured: measured.len(),
    })
}

/// Listed rates and 30-minute statistics in one line, for example
/// `$0.177/$0.355 · 52 tok/s · 0.9 s · 89% off · fp8 · cache $0.0147 · 1M ctx`.
pub fn describe(endpoint: &Endpoint, reasoning: bool) -> String {
    let mut parts = Vec::new();
    if endpoint.supports("tools") == Some(false) {
        parts.push("no tools".to_owned());
    } else if reasoning && endpoint.supports("reasoning") == Some(false) {
        parts.push("no reasoning".to_owned());
    }
    parts.push(match (endpoint.prompt, endpoint.completion) {
        (None, None) => "price ?".into(),
        (input, output) => format!("{}/{}", price(input), price(output)),
    });
    if endpoint.throughput.is_none() && endpoint.latency.is_none() {
        parts.push("no stats".into());
    }
    if let Some(throughput) = endpoint.throughput {
        parts.push(format!("{} tok/s", throughput.round()));
    }
    if let Some(latency) = endpoint.latency {
        parts.push(format!("{:.1} s", latency / 1000.0));
    }
    if let Some(discount) = endpoint.discount.filter(|discount| *discount > 0.0) {
        parts.push(format!("{}% off", (discount * 100.0).round()));
    }
    if let Some(status) = endpoint.status.filter(|status| *status != 0) {
        parts.push(format!("status {status}"));
    }
    if let Some(uptime) = endpoint.uptime.filter(|uptime| *uptime < 99.0) {
        parts.push(format!("up {}%", uptime.round()));
    }
    parts.extend(endpoint.tier.clone());
    parts.extend(endpoint.quantization.clone());
    if let Some(cache) = endpoint.cache_read {
        parts.push(format!("cache {}", price(Some(cache))));
    }
    if let Some(context) = endpoint.context {
        parts.push(format!("{} ctx", quantity(context)));
    }
    parts.join(" · ")
}

/// USD with about three significant digits; `?` when unknown.
pub(super) fn price(value: Option<f64>) -> String {
    let Some(value) = value else {
        return "?".into();
    };
    if value == 0.0 {
        return "$0".into();
    }
    let digits = if value >= 100.0 {
        0
    } else if value >= 1.0 {
        2
    } else {
        (2 - value.log10().floor() as i32).clamp(2, 6) as usize
    };
    let text = format!("{value:.digits$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        &text
    };
    format!("${text}")
}

pub(super) fn quantity(count: usize) -> String {
    let (value, unit) = if count >= 1_000_000 {
        (count as f64 / 1_000_000.0, "M")
    } else if count >= 1_000 {
        (count as f64 / 1_000.0, "K")
    } else {
        return count.to_string();
    };
    let text = if value >= 100.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    };
    format!("{}{unit}", text.trim_end_matches(".0"))
}

fn median(values: impl Iterator<Item = f64>) -> f64 {
    let mut values: Vec<f64> = values.collect();
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

fn round(value: f64, scale: f64) -> f64 {
    (value * scale).round() / scale
}

/// A conversation's routing with the endpoint metadata that informs it.
#[derive(Clone, Debug)]
pub struct Router {
    pub routing: Routing,
    endpoints: Option<Endpoints>,
    /// The model and time of the last metadata fetch, successful or not.
    attempted: Option<(String, Instant)>,
    /// The conversation identifier sent as `session_id` in Auto.
    pub session: Option<String>,
}

impl Router {
    pub fn new(routing: Routing) -> Self {
        Self {
            routing,
            endpoints: None,
            attempted: None,
            session: None,
        }
    }

    /// Metadata for `model`, however old.
    pub fn endpoints(&self, model: &str) -> Option<&Endpoints> {
        self.endpoints
            .as_ref()
            .filter(|endpoints| endpoints.model == model)
    }

    /// Keeps a listing; older listings and other models' listings are ignored.
    pub fn offer(&mut self, endpoints: Endpoints) {
        let newer = self.endpoints.as_ref().is_none_or(|current| {
            current.model != endpoints.model || current.fetched <= endpoints.fetched
        });
        if newer {
            self.attempted = Some((endpoints.model.clone(), endpoints.fetched));
            self.endpoints = Some(endpoints);
        }
    }

    pub fn failed(&mut self, model: &str, now: Instant) {
        self.attempted = Some((model.into(), now));
    }

    /// Whether a turn should fetch metadata first: at most once per freshness
    /// window, and after a failure only once the retry delay has passed.
    pub fn needs_refresh(&self, model: &str, now: Instant) -> bool {
        if self.routing == Routing::Default {
            return false;
        }
        let fresh = self
            .endpoints(model)
            .is_some_and(|endpoints| endpoints.fresh(now));
        let waiting = self.attempted.as_ref().is_some_and(|(attempted, time)| {
            attempted == model && now.saturating_duration_since(*time) < policy::RETRY
        });
        !fresh && !waiting
    }

    pub fn pinned(&self, model: &str) -> Option<&Endpoint> {
        match &self.routing {
            Routing::Endpoint(tag) => self.endpoints(model)?.find(tag),
            _ => None,
        }
    }

    /// A pin needs a fresh inventory before a turn: provider slugs match their
    /// variants too, so an unverified inventory cannot exclude those variants.
    pub fn validate_pin(&self, model: &str, reasoning: bool, now: Instant) -> Result<(), String> {
        let Routing::Endpoint(tag) = &self.routing else {
            return Ok(());
        };
        let endpoints = self.endpoints(model).filter(|endpoints| endpoints.fresh(now))
            .ok_or_else(|| format!(
                "Cannot verify pinned endpoint {tag}: a fresh endpoint listing is required; use /routing to refresh it or choose Auto."
            ))?;
        let endpoint = endpoints.find(tag).ok_or_else(|| format!(
            "Pinned endpoint {tag} is absent from the latest listing; use /routing to choose Auto or another endpoint."
        ))?;
        if !compatible(endpoint, reasoning) {
            return Err(format!(
                "Pinned endpoint {tag} does not support the current tool or reasoning requirements; use /routing or /effort to change them."
            ));
        }
        Ok(())
    }

    /// Whether requests may use a parameter: false only when a pinned
    /// endpoint is known not to accept it.
    pub fn allows(&self, model: &str, parameter: &str) -> bool {
        self.pinned(model)
            .and_then(|endpoint| endpoint.supports(parameter))
            != Some(false)
    }

    /// Model limits narrowed to a pinned endpoint's known limits.
    pub fn limits(&self, model: &str, limits: Option<Limits>) -> Option<Limits> {
        let Some(endpoint) = self.pinned(model) else {
            return limits;
        };
        let context = match (limits.map(|limits| limits.context), endpoint.context) {
            (Some(model), Some(endpoint)) => model.min(endpoint),
            (model, endpoint) => model.or(endpoint)?,
        };
        Some(Limits {
            context,
            output: endpoint
                .output
                .or_else(|| limits.and_then(|limits| limits.output)),
        })
    }

    pub fn guard(&self, model: &str, reasoning: bool, now: Instant) -> Option<Guard> {
        self.endpoints(model)
            .filter(|endpoints| endpoints.fresh(now))
            .and_then(|endpoints| guard(endpoints, reasoning))
    }

    /// Top-level request fields for `model`'s next request.
    pub fn fields(&self, model: &str, reasoning: bool, now: Instant) -> Vec<(&'static str, Value)> {
        match &self.routing {
            Routing::Default => vec![],
            Routing::Auto => {
                let mut fields = Vec::new();
                if let Some(session) = &self.session {
                    fields.push(("session_id", Value::string(session)));
                }
                if let Some(guard) = self.guard(model, reasoning, now) {
                    fields.push((
                        "provider",
                        Value::object([
                            ("preferred_min_throughput", Value::number(guard.throughput)),
                            ("preferred_max_latency", Value::number(guard.latency)),
                        ]),
                    ));
                }
                fields
            }
            Routing::Endpoint(tag) => {
                // A slug also matches longer slugs that extend it, such as
                // regions of a variant; ignoring the known ones keeps the pin exact.
                let extensions: Vec<_> = self
                    .endpoints(model)
                    .into_iter()
                    .flat_map(|endpoints| &endpoints.list)
                    .filter(|endpoint| {
                        endpoint
                            .tag
                            .strip_prefix(tag.as_str())
                            .is_some_and(|rest| rest.starts_with('/'))
                    })
                    .map(|endpoint| Value::string(&endpoint.tag))
                    .collect();
                let mut provider = Value::object([
                    ("order", Value::Array(vec![Value::string(tag)])),
                    ("allow_fallbacks", Value::Bool(false)),
                ]);
                if !extensions.is_empty()
                    && let Value::Object(fields) = &mut provider
                {
                    fields.insert("ignore".into(), Value::Array(extensions));
                }
                vec![("provider", provider)]
            }
        }
    }
}

/// One line on what `routing` does with the guard fresh metadata supports.
pub fn explain(routing: &Routing, guard: Option<Guard>) -> String {
    match routing {
        Routing::Auto => match guard {
            Some(guard) => format!(
                "Native routing with fallbacks · prefers ≥ {} tok/s and ≤ {} s to first token, from {} measured endpoints (provisional)",
                guard.throughput, guard.latency, guard.measured
            ),
            None => {
                "Native routing with fallbacks · too little fresh performance data for speed preferences"
                    .into()
            }
        },
        Routing::Endpoint(tag) => {
            format!("Pinned to {tag} without fallbacks · requests fail if it is unavailable")
        }
        Routing::Default => {
            "Saved before routing options existed · no routing preferences are sent".into()
        }
    }
}

#[cfg(test)]
mod tests;
