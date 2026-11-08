//! Model catalogs for ChatGPT accounts and direct OpenAI API connections.

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

/// The public Models API supplies IDs and creation times but no context or
/// reasoning capability data. Exact documented model limits fill that gap;
/// unrecognized IDs remain unknown. Its order is unspecified, so newest come first.
pub(super) fn parse_api(response: &Value) -> Result<Catalog, String> {
    let rows = response
        .get("data")
        .and_then(Value::as_array)
        .ok_or("OpenAI returned an invalid API model list")?;
    let mut seen = BTreeSet::new();
    let mut dated = Vec::new();
    let mut limits = BTreeMap::new();
    for row in rows {
        let Some(id) = row.get("id").and_then(Value::as_str) else {
            continue;
        };
        if validate_model(id).is_err() || other_api_endpoint(id) || !seen.insert(id.to_owned()) {
            continue;
        }
        if let Some(limit) = documented_api_limits(id) {
            limits.insert(id.into(), limit);
        }
        let created = row.get("created").and_then(Value::as_usize).unwrap_or(0);
        dated.push((
            created,
            Model {
                id: id.into(),
                name: id.into(),
                prompt_price: None,
                completion_price: None,
                efforts: api_efforts(id),
            },
        ));
    }
    dated.sort_by_key(|(created, _)| std::cmp::Reverse(*created));
    Ok(Catalog {
        models: dated.into_iter().map(|(_, model)| model).collect(),
        limits,
    })
}

/// Current exact API model IDs from developers.openai.com/api/docs/models.
/// Subscription windows come from account metadata and can differ from these.
fn documented_api_limits(id: &str) -> Option<Limits> {
    match id {
        "gpt-6-astra" | "gpt-6.1-sol" | "gpt-6-sol" | "gpt-6-luna" | "gpt-6.1-terra" => {
            Some(Limits {
                context: 1_050_000,
                output: Some(128_000),
            })
        }
        _ => None,
    }
}

/// The public catalog mixes endpoints. Omit established non-Responses families;
/// leave unrecognized IDs available rather than guessing their capabilities.
fn other_api_endpoint(id: &str) -> bool {
    [
        "text-embedding-",
        "text-moderation-",
        "omni-moderation-",
        "dall-e-",
        "gpt-image-",
        "chatgpt-image-",
        "whisper-",
        "tts-",
        "gpt-audio",
        "gpt-realtime",
        "gpt-live",
        "gpt-transcribe",
        "gpt-4o-audio-",
        "gpt-4o-mini-audio-",
        "gpt-4o-realtime-",
        "gpt-4o-mini-realtime-",
        "gpt-4o-transcribe",
        "gpt-4o-mini-transcribe",
        "gpt-4o-tts",
        "gpt-4o-mini-tts",
    ]
    .iter()
    .any(|prefix| id.starts_with(prefix))
}

/// The public Models API has no effort metadata. Keep only exact model IDs
/// whose levels are documented on their current OpenAI API model pages.
fn api_efforts(id: &str) -> Vec<Effort> {
    let documented: &[Effort] = match id {
        "gpt-6-astra" | "gpt-6.1-sol" => &[
            Effort::Low,
            Effort::Medium,
            Effort::High,
            Effort::Xhigh,
            Effort::Max,
        ],
        "gpt-6-sol" | "gpt-6-luna" | "gpt-6.1-terra" => &[
            Effort::None,
            Effort::Low,
            Effort::Medium,
            Effort::High,
            Effort::Xhigh,
            Effort::Max,
        ],
        _ => &[],
    };
    let mut efforts = vec![Effort::Default];
    efforts.extend_from_slice(documented);
    efforts
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
    fn api_catalog_uses_only_documented_exact_model_efforts() {
        let response = crate::json::parse(
            r#"{"data":[{"id":"gpt-6-astra"},{"id":"gpt-6.1-sol"},{"id":"gpt-6-sol"},{"id":"gpt-6-luna"},{"id":"gpt-6.1-terra"},{"id":"gpt-6.1-sol-2026-11-01"},{"id":"other"}]}"#,
        )
        .unwrap();
        let models = parse_api(&response).unwrap().models;
        assert_eq!(models.len(), 7);
        for model in &models[..2] {
            assert_eq!(
                model.efforts,
                [
                    Effort::Default,
                    Effort::Low,
                    Effort::Medium,
                    Effort::High,
                    Effort::Xhigh,
                    Effort::Max
                ]
            );
        }
        for model in &models[2..5] {
            assert_eq!(
                model.efforts,
                [
                    Effort::Default,
                    Effort::None,
                    Effort::Low,
                    Effort::Medium,
                    Effort::High,
                    Effort::Xhigh,
                    Effort::Max
                ]
            );
        }
        assert_eq!(models[5].efforts, [Effort::Default]);
        assert_eq!(models[6].efforts, [Effort::Default]);
    }

    #[test]
    fn api_catalog_uses_documented_windows_for_exact_models_only() {
        let response = crate::json::parse(
            r#"{"data":[{"id":"gpt-6-astra"},{"id":"gpt-6.1-sol"},{"id":"gpt-6-sol"},{"id":"gpt-6-luna"},{"id":"gpt-6.1-terra"},{"id":"gpt-6.1-sol-2026-11-01"},{"id":"unknown-model"}]}"#,
        )
        .unwrap();
        let catalog = parse_api(&response).unwrap();
        for id in [
            "gpt-6-astra",
            "gpt-6.1-sol",
            "gpt-6-sol",
            "gpt-6-luna",
            "gpt-6.1-terra",
        ] {
            let limits = catalog.limits.get(id).unwrap();
            assert_eq!(limits.context, 1_050_000);
            assert_eq!(limits.output, Some(128_000));
        }
        assert!(!catalog.limits.contains_key("gpt-6.1-sol-2026-11-01"));
        assert!(!catalog.limits.contains_key("unknown-model"));
    }

    #[test]
    fn api_catalog_lists_newest_models_first_and_keeps_ties_in_order() {
        let response = crate::json::parse(
            r#"{"data":[{"id":"undated"},{"id":"older","created":100},{"id":"newest","created":300},{"id":"tie-a","created":200},{"id":"tie-b","created":200}]}"#,
        )
        .unwrap();
        let ids = parse_api(&response)
            .unwrap()
            .models
            .into_iter()
            .map(|model| model.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, ["newest", "tie-a", "tie-b", "older", "undated"]);
    }

    #[test]
    fn api_catalog_omits_other_endpoint_families_and_keeps_unknown_models() {
        let response = crate::json::parse(
            r#"{"data":[
                {"id":"text-embedding-3-small"},
                {"id":"text-embedding-ada-002"},
                {"id":"omni-moderation-latest"},
                {"id":"text-moderation-stable"},
                {"id":"dall-e-3"},
                {"id":"gpt-image-2"},
                {"id":"chatgpt-image-latest"},
                {"id":"whisper-1"},
                {"id":"tts-1-hd"},
                {"id":"gpt-audio-1.5"},
                {"id":"gpt-realtime-2.1"},
                {"id":"gpt-live-1"},
                {"id":"gpt-transcribe"},
                {"id":"gpt-4o-audio-preview-2025-06-03"},
                {"id":"gpt-4o-mini-audio-preview"},
                {"id":"gpt-4o-realtime-preview"},
                {"id":"gpt-4o-mini-realtime-preview"},
                {"id":"gpt-4o-transcribe-diarize"},
                {"id":"gpt-4o-mini-transcribe"},
                {"id":"gpt-4o-mini-tts"},
                {"id":"gpt-6.1-sol","created":300},
                {"id":"ft:gpt-4.1:fixture:custom","created":200},
                {"id":"future-model","created":100}
            ]}"#,
        )
        .unwrap();
        let ids = parse_api(&response)
            .unwrap()
            .models
            .into_iter()
            .map(|model| model.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            ["gpt-6.1-sol", "ft:gpt-4.1:fixture:custom", "future-model"]
        );
    }

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
