//! Account-scoped model capabilities returned by the ChatGPT model catalog.

use crate::backend::{Limits, Model, validate_model};
use crate::effort::Effort;
use crate::json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Catalog {
    pub models: Vec<Model>,
    pub limits: BTreeMap<String, Limits>,
}

pub(super) fn parse(response: &Value) -> Result<Catalog, String> {
    let rows = response
        .get("models")
        .and_then(Value::as_array)
        .ok_or("OpenAI returned an invalid ChatGPT model catalog")?;
    let mut models = Vec::new();
    let mut limits = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for row in rows {
        if row.get("visibility").and_then(Value::as_str) != Some("list") {
            continue;
        }
        let Some(slug) = row.get("slug").and_then(Value::as_str) else {
            continue;
        };
        if validate_model(slug).is_err() || !seen.insert(slug.to_owned()) {
            continue;
        }
        let name = row
            .get("display_name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(slug);
        models.push(Model {
            id: slug.into(),
            name: name.into(),
            prompt_price: None,
            completion_price: None,
            efforts: efforts(row),
        });
        if let Some(context) = row.get("context_window").and_then(Value::as_usize)
            && context > 0
        {
            limits.insert(
                slug.into(),
                Limits {
                    context,
                    output: None,
                },
            );
        }
    }
    Ok(Catalog { models, limits })
}

fn efforts(row: &Value) -> Vec<Effort> {
    let mut result = vec![Effort::Default];
    if let Some(levels) = row
        .get("supported_reasoning_levels")
        .and_then(Value::as_array)
    {
        for level in levels {
            if let Some(name) = level.get("effort").and_then(Value::as_str)
                && let Ok(effort) = Effort::parse(name)
                && !result.contains(&effort)
            {
                result.push(effort);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_catalog_keeps_order_efforts_and_effective_context() {
        // Sanitized shape of the account catalog. max_context_window is not
        // the currently available limit, and no output cap is declared.
        let response = crate::json::parse(
            r#"{"models":[{"slug":"gpt-6.1-sol","display_name":"GPT-6.1-Sol","visibility":"list","context_window":500000,"max_context_window":872000,"supported_reasoning_levels":[{"effort":"low"},{"effort":"medium"},{"effort":"high"},{"effort":"xhigh"},{"effort":"max"},{"effort":"ultra"}],"default_reasoning_level":"low"},{"slug":"hidden","visibility":"hidden","context_window":999999},{"slug":"second","visibility":"list","supported_reasoning_levels":[{"effort":"medium"}]},{"slug":"gpt-6.1-sol","visibility":"list","context_window":1}]}"#,
        )
        .unwrap();
        let catalog = parse(&response).unwrap();
        assert_eq!(
            catalog
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["gpt-6.1-sol", "second"]
        );
        assert_eq!(
            catalog.models[0].efforts,
            [
                Effort::Default,
                Effort::Low,
                Effort::Medium,
                Effort::High,
                Effort::Xhigh,
                Effort::Max,
                Effort::Ultra,
            ]
        );
        assert_eq!(catalog.models[1].efforts, [Effort::Default, Effort::Medium]);
        let limit = catalog.limits.get("gpt-6.1-sol").unwrap();
        assert_eq!(limit.context, 500_000);
        assert_eq!(limit.output, None);
        assert!(!catalog.limits.contains_key("second"));
    }
}
