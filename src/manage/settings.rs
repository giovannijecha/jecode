//! `jecode settings`: the saved defaults `/settings` changes, through the
//! same parsers and `Store::save`. Values apply to new conversations; theme,
//! auto-compact and prompt-cache also reach running ones, as in the TUI.

use super::{Context, Report};
use crate::backend::{CacheRetention, Routing};
use crate::cli::{Failure, Manage, usage};
use crate::config::{Settings, Theme};
use crate::effort::Effort;
use crate::json::Value;
use crate::permissions::Level;

pub const KEYS: &[&str] = &[
    "model",
    "effort",
    "routing",
    "permissions",
    "auto-compact",
    "prompt-cache",
    "theme",
];

pub fn run(context: &Context, command: &Manage) -> Result<Report, Failure> {
    match command.action {
        "show" => show(context),
        "get" => get(context, &command.operands[0]),
        "set" => set(
            context,
            &command.operands[0],
            &command.operands[1],
            command.value("provider"),
        ),
        "path" => Ok(Report::new(
            Value::object([
                (
                    "config",
                    Value::string(context.store.path().to_string_lossy()),
                ),
                ("home", Value::string(context.home.to_string_lossy())),
            ]),
            format!(
                "Configuration: {}\nJecode home:   {}",
                context.store.path().display(),
                context.home.display()
            ),
        )),
        action => unreachable!("settings {action}"),
    }
}

fn current(context: &Context) -> Result<(Settings, bool), Failure> {
    Ok(match context.store.load()? {
        Some(settings) => (settings, true),
        None => (Settings::unconfigured(), false),
    })
}

fn provider_name(id: &str) -> &str {
    crate::backend::info(id).map_or(id, |info| info.name)
}

/// Each key's value as text and JSON, never including a credential.
fn values(settings: &Settings, configured: bool) -> Vec<(&'static str, String, Value)> {
    let model = configured.then(|| settings.model.clone());
    let routing = if settings.backend_id == "openrouter" && configured {
        settings.routing_for(&settings.model)
    } else {
        Routing::Default
    };
    vec![
        (
            "provider",
            if configured {
                format!(
                    "{} ({})",
                    settings.backend_id,
                    provider_name(&settings.backend_id)
                )
            } else {
                "not configured".into()
            },
            if configured {
                Value::string(&settings.backend_id)
            } else {
                Value::Null
            },
        ),
        (
            "model",
            model.clone().unwrap_or_else(|| "not configured".into()),
            Value::optional(model),
        ),
        (
            "effort",
            settings.effort.name().into(),
            Value::string(settings.effort.name()),
        ),
        ("routing", routing_text(&routing), routing_value(&routing)),
        (
            "permissions",
            settings.default_permissions.id().into(),
            Value::string(settings.default_permissions.id()),
        ),
        (
            "auto-compact",
            format!("{}%", settings.auto_compact),
            Value::number(settings.auto_compact),
        ),
        (
            "prompt-cache",
            settings.cache_retention.id().into(),
            Value::string(settings.cache_retention.id()),
        ),
        (
            "theme",
            settings.theme.id().into(),
            Value::string(settings.theme.id()),
        ),
    ]
}

fn routing_text(routing: &Routing) -> String {
    match routing {
        Routing::Auto => "auto".into(),
        Routing::Endpoint(tag) => tag.clone(),
        Routing::Default => "not applicable".into(),
    }
}

fn routing_value(routing: &Routing) -> Value {
    match routing {
        Routing::Default => Value::Null,
        routing => Value::string(routing_text(routing)),
    }
}

fn show(context: &Context) -> Result<Report, Failure> {
    let (settings, configured) = current(context)?;
    let rows = values(&settings, configured);
    let mut text = String::new();
    for (key, shown, _) in &rows {
        text.push_str(&format!("{key:<13} {shown}\n"));
    }
    text.push_str(&format!("\nSaved in {}", context.store.path().display()));
    if !configured {
        text.push_str(" (not created yet; run jecode setup)");
    }
    let value = Value::object([
        (
            "path",
            Value::string(context.store.path().to_string_lossy()),
        ),
        ("configured", Value::Bool(configured)),
        (
            "settings",
            Value::entries(rows.into_iter().map(|(key, _, value)| (key, value))),
        ),
    ]);
    Ok(Report::new(value, text))
}

fn get(context: &Context, key: &str) -> Result<Report, Failure> {
    let (settings, configured) = current(context)?;
    let rows = values(&settings, configured);
    let (key, shown, value) = rows
        .into_iter()
        .find(|(name, _, _)| *name == key)
        .ok_or_else(|| unknown(key))?;
    Ok(Report::new(
        Value::object([("key", Value::string(key)), ("value", value)]),
        shown,
    ))
}

fn unknown(key: &str) -> Failure {
    usage(format!(
        "unknown setting '{key}'; use provider (read only), {}",
        KEYS.join(", ")
    ))
}

fn set(
    context: &Context,
    key: &str,
    value: &str,
    provider: Option<&str>,
) -> Result<Report, Failure> {
    if !KEYS.contains(&key) {
        return Err(if key == "provider" {
            usage(
                "the provider changes with its model: jecode settings set model ID --provider PROVIDER",
            )
        } else {
            unknown(key)
        });
    }
    if provider.is_some() && key != "model" {
        return Err(usage("--provider applies only to settings set model"));
    }
    let mut settings = context.store.load()?.ok_or_else(|| {
        Failure::Error(
            "No saved configuration yet. Run jecode setup to choose a provider and default model first."
                .into(),
        )
    })?;
    let before = values(&settings, true)
        .into_iter()
        .find(|(name, _, _)| *name == key)
        .map(|(_, shown, _)| shown)
        .unwrap_or_default();
    let services = |settings: &Settings, id: &str| {
        crate::selection::services(settings, &context.home, id).map_err(Failure::Error)
    };
    match key {
        "model" => {
            crate::config::validate_model(value).map_err(usage)?;
            let provider = provider.unwrap_or(&settings.backend_id).to_owned();
            services(&settings, &provider)?;
            let same = provider == settings.backend_id && value == settings.model;
            if provider != settings.backend_id {
                let key = settings.credential_for(&provider).unwrap_or("").to_owned();
                settings.select_backend(&provider, key)?;
            }
            settings.model = value.to_owned();
            if !same {
                settings.effort = Effort::Default;
            }
        }
        "effort" => {
            let effort = Effort::parse(value).map_err(usage)?;
            let services = services(&settings, &settings.backend_id)?;
            crate::selection::check_effort(&services, &settings.model, effort)?;
            settings.effort = effort;
        }
        "routing" => {
            if settings.backend_id != "openrouter" {
                return Err(usage(format!(
                    "routing applies to OpenRouter; the default provider is {}",
                    settings.backend_id
                )));
            }
            let routing = if value == "auto" {
                Routing::Auto
            } else {
                let services = services(&settings, &settings.backend_id)?;
                let endpoints = services
                    .endpoints(&settings.model)?
                    .ok_or("Endpoint routing is unavailable for this provider")?;
                if endpoints.find(value).is_none() {
                    return Err(Failure::Error(format!(
                        "{} has no endpoint '{value}'; choose one in /settings > Default routing, or use auto",
                        settings.model
                    )));
                }
                Routing::Endpoint(value.to_owned())
            };
            settings.routing.insert(settings.model.clone(), routing);
        }
        "permissions" => {
            settings.default_permissions = Level::parse(value)
                .ok_or_else(|| usage("permissions must be read, workspace or full"))?;
        }
        "auto-compact" => {
            let number = value
                .trim()
                .trim_end_matches('%')
                .parse::<usize>()
                .map_err(|_| usage("auto-compact must be a whole percentage from 1 to 99"))?;
            settings.auto_compact = crate::context::auto_compact_percent(number).map_err(usage)?;
        }
        "prompt-cache" => {
            settings.cache_retention = CacheRetention::parse(value).map_err(usage)?;
        }
        "theme" => settings.theme = Theme::parse(value).map_err(usage)?,
        _ => unreachable!(),
    }
    context.store.save(&settings)?;
    let saved = context
        .store
        .load()?
        .ok_or("Saved configuration could not be reopened")?;
    let rows = values(&saved, true);
    let (_, after, value) = rows
        .iter()
        .find(|(name, _, _)| *name == key)
        .cloned()
        .unwrap();
    let mut text = format!("{key}: {before} → {after} · saved for new conversations");
    if key == "model" {
        text.push_str(&format!(
            " · provider {} · effort {}",
            saved.backend_id,
            saved.effort.name()
        ));
    }
    Ok(Report::new(
        Value::object([
            ("key", Value::string(key)),
            ("previous", Value::string(before)),
            ("value", value),
            ("provider", Value::string(&saved.backend_id)),
        ]),
        text,
    ))
}
