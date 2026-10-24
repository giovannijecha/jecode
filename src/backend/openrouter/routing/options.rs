//! The routing choices offered for one model, shared by every interface.

use super::{Routing, compatible, describe, explain, facts, guard, metrics};
use crate::backend::Endpoints;
use crate::backend::Entry;

/// Auto first, then listed endpoints in OpenRouter's order with compatible
/// ones first, then a current choice the listing does not show.
pub fn entries(endpoints: Option<&Endpoints>, current: &Routing, reasoning: bool) -> Vec<Entry> {
    let mark = |routing: &Routing, description: String| {
        if routing == current {
            format!("current · {description}")
        } else {
            description
        }
    };
    let mut entries = vec![Entry {
        routing: Routing::Auto,
        name: Routing::Auto.label(),
        description: mark(&Routing::Auto, "native routing with fallbacks".into()),
        metrics: None,
        facts: String::new(),
        help: explain(
            &Routing::Auto,
            endpoints.and_then(|endpoints| guard(endpoints, reasoning)),
        ),
        problem: None,
    }];
    let mut listed: Vec<_> = endpoints
        .map_or(&[][..], |endpoints| &endpoints.list)
        .iter()
        .collect();
    listed.sort_by_key(|endpoint| !compatible(endpoint, reasoning));
    for endpoint in listed {
        let routing = Routing::Endpoint(endpoint.tag.clone());
        entries.push(Entry {
            name: endpoint.tag.clone(),
            description: mark(&routing, describe(endpoint, reasoning)),
            metrics: Some(metrics(endpoint)),
            facts: facts(endpoint, reasoning),
            help: explain(&routing, None),
            problem: if endpoint.supports("tools") == Some(false) {
                Some(format!(
                    "{} does not accept tool calls, which every Jecode request includes",
                    endpoint.tag
                ))
            } else if reasoning && endpoint.supports("reasoning") == Some(false) {
                Some(format!(
                    "{} does not accept reasoning required by the current effort",
                    endpoint.tag
                ))
            } else {
                None
            },
            routing,
        });
    }
    match current {
        Routing::Endpoint(tag) if !entries.iter().any(|entry| entry.routing == *current) => {
            let fact = if endpoints.is_some() {
                "not in the latest listing"
            } else {
                "listing unavailable"
            };
            entries.push(Entry {
                routing: current.clone(),
                name: tag.clone(),
                description: mark(current, fact.into()),
                metrics: None,
                facts: fact.into(),
                help: explain(current, None),
                problem: None,
            });
        }
        Routing::Default => entries.push(Entry {
            routing: Routing::Default,
            name: Routing::Default.label(),
            description: mark(current, "no routing preferences".into()),
            metrics: None,
            facts: "no routing preferences".into(),
            help: explain(current, None),
            problem: None,
        }),
        _ => {}
    }
    entries
}
