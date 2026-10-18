//! The routing choices offered for one model, shared by every interface.

use super::{Routing, compatible, describe, explain, guard};
use crate::openrouter::endpoints::Endpoints;

/// One selectable routing with the words that present it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub routing: Routing,
    pub name: String,
    /// Prefixed with `current · ` for the routing in effect.
    pub description: String,
    /// What choosing it does.
    pub help: String,
    /// Why every request would fail with it, when the listing shows that.
    pub problem: Option<String>,
}

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
            entries.push(Entry {
                routing: current.clone(),
                name: tag.clone(),
                description: mark(
                    current,
                    if endpoints.is_some() {
                        "not in the latest listing".into()
                    } else {
                        "listing unavailable".into()
                    },
                ),
                help: explain(current, None),
                problem: None,
            });
        }
        Routing::Default => entries.push(Entry {
            routing: Routing::Default,
            name: Routing::Default.label(),
            description: mark(current, "no routing preferences".into()),
            help: explain(current, None),
            problem: None,
        }),
        _ => {}
    }
    entries
}
