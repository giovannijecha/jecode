//! Account model metadata; unknown capabilities stay unknown.

use crate::backend::{Limits, Model, validate_model};
use crate::effort::Effort;
use crate::json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Catalog {
    pub models: Vec<Model>,
    pub limits: BTreeMap<String, Limits>,
    pub thinking_off: BTreeMap<String, &'static str>,
}

pub(super) fn parse(response: &Value) -> Result<Catalog, String> {
    let rows = response
        .get("data")
        .and_then(Value::as_array)
        .ok_or("Anthropic returned an invalid model catalog")?;
    let mut catalog = Catalog::default();
    let mut seen = BTreeSet::new();
    for row in rows {
        let Some(id) = row.get("id").and_then(Value::as_str) else {
            continue;
        };
        if validate_model(id).is_err() || !seen.insert(id.to_owned()) {
            continue;
        }
        let name = row
            .get("display_name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(id);
        let effort = row.get("capabilities").and_then(|v| v.get("effort"));
        let mut efforts = vec![Effort::Default];
        if let Some(effort) = effort {
            for (name, level) in [
                ("low", Effort::Low),
                ("medium", Effort::Medium),
                ("high", Effort::High),
                ("xhigh", Effort::Xhigh),
                ("max", Effort::Max),
            ] {
                if effort.get(name).and_then(|value| value.get("supported"))
                    == Some(&Value::Bool(true))
                {
                    efforts.push(level);
                }
            }
        }
        let thinking_types = row
            .get("capabilities")
            .and_then(|v| v.get("thinking"))
            .and_then(|v| v.get("types"));
        let thinking_off = ["between_tools", "disabled"].into_iter().find(|mode| {
            thinking_types
                .and_then(|types| types.get(mode))
                .and_then(|value| value.get("supported"))
                == Some(&Value::Bool(true))
        });
        if let Some(mode) = thinking_off {
            efforts.push(Effort::None);
            catalog.thinking_off.insert(id.into(), mode);
        }
        catalog.models.push(Model {
            id: id.into(),
            name: name.into(),
            prompt_price: None,
            completion_price: None,
            efforts,
        });
        if let Some(context) = row.get("max_input_tokens").and_then(Value::as_usize)
            && context > 0
        {
            catalog.limits.insert(
                id.into(),
                Limits {
                    context,
                    output: row
                        .get("max_tokens")
                        .and_then(Value::as_usize)
                        .filter(|output| *output > 0),
                },
            );
        }
    }
    Ok(catalog)
}
