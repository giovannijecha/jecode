//! The last context count the selected provider actually reported.

use crate::{backend::Usage, conversation::Origin, json::Value};
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Observation {
    origin: Origin,
    tokens: usize,
}

/// A saved view keeps its last observation; changing the view cannot change usage.
#[derive(Debug, Default)]
pub(crate) struct Counter(Mutex<Option<Observation>>);

impl Clone for Counter {
    fn clone(&self) -> Self {
        Self(Mutex::new(self.0.lock().unwrap().clone()))
    }
}

impl Counter {
    pub fn observe(&self, origin: Origin, usage: &Usage) -> bool {
        let Some(tokens) = usage.context_tokens() else {
            return false;
        };
        let observation = Some(Observation { origin, tokens });
        let mut current = self.0.lock().unwrap();
        if *current == observation {
            return false;
        }
        *current = observation;
        true
    }

    pub fn tokens_for(&self, origin: &Origin) -> Option<usize> {
        self.0
            .lock()
            .unwrap()
            .as_ref()
            .filter(|observed| &observed.origin == origin)
            .map(|observed| observed.tokens)
    }

    pub fn value(&self) -> Value {
        self.0
            .lock()
            .unwrap()
            .as_ref()
            .map_or(Value::Null, |observed| {
                Value::object([
                    ("origin", observed.origin.value()),
                    ("tokens", Value::number(observed.tokens)),
                ])
            })
    }

    pub fn parse(value: Option<&Value>) -> Result<Self, String> {
        let Some(value) = value.filter(|value| **value != Value::Null) else {
            return Ok(Self::default());
        };
        let observed = Observation {
            origin: Origin::parse(value.get("origin").ok_or("Missing provider count origin")?)?,
            tokens: value
                .get("tokens")
                .and_then(Value::as_usize)
                .ok_or("Invalid provider context count")?,
        };
        Ok(Self(Mutex::new(Some(observed))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_provider_counts_can_create_or_replace_an_observation() {
        let counter = Counter::default();
        let origin = Origin::new("openai", "fixture");
        assert_eq!(counter.tokens_for(&origin), None);
        assert!(!counter.observe(origin.clone(), &Usage::default()));
        assert!(counter.observe(
            origin.clone(),
            &Usage {
                input_tokens: Some(1000),
                output_tokens: Some(200),
                cached_tokens: Some(800),
                ..Usage::default()
            }
        ));
        assert_eq!(counter.tokens_for(&origin), Some(1200));
        assert!(!counter.observe(
            origin.clone(),
            &Usage {
                output_tokens: Some(50),
                ..Usage::default()
            }
        ));
        assert_eq!(counter.tokens_for(&origin), Some(1200));
        assert!(counter.observe(
            origin.clone(),
            &Usage {
                total_tokens: Some(1050),
                ..Usage::default()
            }
        ));
        assert_eq!(counter.tokens_for(&origin), Some(1050));
    }

    #[test]
    fn projection_clones_keep_the_observation_but_identity_changes_do_not_share_it() {
        let origin = Origin::new("openai", "fixture");
        let mut context = crate::context::Context::default();
        context.reported.observe(
            origin.clone(),
            &Usage {
                total_tokens: Some(4000),
                ..Usage::default()
            },
        );
        let mut preview = context.clone();
        preview.clear_measurement();
        assert_eq!(preview.reported.tokens_for(&origin), Some(4000));
        context.reported.observe(
            origin.clone(),
            &Usage {
                total_tokens: Some(8000),
                ..Usage::default()
            },
        );
        assert_eq!(preview.reported.tokens_for(&origin), Some(4000));
        context.reset_usage();
        assert_eq!(context.reported.tokens_for(&origin), None);
        assert_eq!(preview.reported.tokens_for(&origin), Some(4000));
    }

    #[test]
    fn saved_observations_are_scoped_to_the_exact_model_and_account() {
        let origin = Origin::with_account("openai", "fixture", Some("account-a".into()));
        let counter = Counter::default();
        counter.observe(
            origin.clone(),
            &Usage {
                total_tokens: Some(0),
                ..Usage::default()
            },
        );
        let restored = Counter::parse(Some(&counter.value())).unwrap();
        assert_eq!(restored.tokens_for(&origin), Some(0));
        assert_eq!(
            restored.tokens_for(&Origin::with_account(
                "openai",
                "fixture",
                Some("account-b".into())
            )),
            None
        );
        assert_eq!(
            restored.tokens_for(&Origin::with_account(
                "openai",
                "other",
                Some("account-a".into())
            )),
            None
        );
    }
}
