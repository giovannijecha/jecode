//! Presentation and aggregation of reported usage without estimating charges.

use super::Charges;

impl Charges {
    /// Adds a reported attempt, including one whose completion was rejected.
    pub fn include(&mut self, other: &Self) {
        for provider in &other.providers {
            if !self
                .providers
                .iter()
                .any(|known| known.eq_ignore_ascii_case(provider))
            {
                self.providers.push(provider.clone());
            }
        }
        self.requests += other.requests;
        self.priced += other.priced;
        self.cost += other.cost;
        self.cached += other.cached;
        self.cache_written += other.cache_written;
        self.byok |= other.byok;
        self.upstream += other.upstream;
    }

    pub fn describe(&self) -> Option<String> {
        let mut parts = Vec::new();
        if !self.providers.is_empty() {
            let mut names = self.providers[..self.providers.len().min(2)].join(", ");
            if self.providers.len() > 2 {
                names.push_str(&format!(" +{}", self.providers.len() - 2));
            }
            parts.push(format!("Served by {names}"));
        }
        if self.priced > 0 {
            let mut cost = format!("{} charged", price(Some(self.cost)));
            if self.priced < self.requests {
                cost.push_str(&format!(
                    " for {} of {} requests",
                    self.priced, self.requests
                ));
            }
            parts.push(cost);
        }
        if self.byok {
            parts.push(if self.upstream > 0.0 {
                format!("BYOK {} upstream", price(Some(self.upstream)))
            } else {
                "BYOK".into()
            });
        }
        if self.cached > 0 {
            parts.push(format!("{} cached", quantity(self.cached)));
        }
        if self.cache_written > 0 {
            parts.push(format!(
                "{} cache write tokens",
                quantity(self.cache_written)
            ));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

/// USD with about three significant digits; `?` when unknown.
pub(crate) fn price(value: Option<f64>) -> String {
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

pub(crate) fn quantity(count: usize) -> String {
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
