//! `jecode models`: the catalog `/model` and Settings > Default model offer,
//! read with the provider's saved credentials. Nothing is saved.

use super::{Context, Report};
use crate::backend::{Model, Routing, Services};
use crate::cli::{Failure, Manage, usage};
use crate::config::Settings;
use crate::json::Value;

#[cfg(test)]
mod tests;

pub fn run(context: &Context, command: &Manage) -> Result<Report, Failure> {
    let settings = context.store.load()?.unwrap_or_else(Settings::unconfigured);
    let provider = command
        .value("provider")
        .map_or_else(|| settings.backend_id.clone(), str::to_owned);
    if command.action == "endpoints" && provider != "openrouter" {
        return Err(usage(format!(
            "endpoint routing is available only for openrouter, not {provider}"
        )));
    }
    let services = crate::selection::services(&settings, &context.home, &provider)?;
    if command.action == "endpoints" {
        return endpoints(&services, &command.operands[0]);
    }
    let catalog = services
        .catalog()
        .map_err(|error| services.redact(&error))?;
    let default =
        (settings.backend_id == provider && settings.has_model()).then(|| settings.model.clone());
    match command.action {
        "list" => {
            let mut text = String::new();
            for model in &catalog {
                text.push_str(&format!(
                    "{}{}  {}\n",
                    model.id,
                    if Some(&model.id) == default.as_ref() {
                        " (default)"
                    } else {
                        ""
                    },
                    prices(model)
                ));
            }
            text.push_str(&format!("{} models from {provider}", catalog.len()));
            Ok(Report::new(
                Value::object([
                    ("provider", Value::string(&provider)),
                    (
                        "models",
                        Value::Array(catalog.iter().map(|model| value(model, &default)).collect()),
                    ),
                ]),
                text,
            ))
        }
        "show" => {
            let id = &command.operands[0];
            let model = catalog
                .iter()
                .find(|model| &model.id == id)
                .ok_or_else(|| {
                    Failure::Error(format!(
                        "{id} is not in the tool-capable catalog of {provider}; list it with jecode models list --provider {provider}"
                    ))
                })?;
            let efforts = model
                .efforts
                .iter()
                .map(|effort| effort.name())
                .collect::<Vec<_>>()
                .join(", ");
            Ok(Report::new(
                value(model, &default),
                format!(
                    "{}\nName:     {}\nProvider: {provider}\nEfforts:  {}\nPrices:   {}",
                    model.id,
                    model.name,
                    if efforts.is_empty() {
                        "default"
                    } else {
                        &efforts
                    },
                    prices(model)
                ),
            ))
        }
        action => unreachable!("models {action}"),
    }
}

fn endpoints(services: &Services, model: &str) -> Result<Report, Failure> {
    crate::config::validate_model(model).map_err(usage)?;
    let listing = services
        .endpoints(model)
        .map_err(|error| services.redact(&error))?
        .ok_or("Endpoint routing is unavailable for this provider")?;
    let choices = |reasoning| {
        services
            .routing_entries(Some(&listing), &Routing::Auto, reasoning)
            .ok_or("Endpoint routing is unavailable for this provider")
    };
    let ordinary = choices(false)?;
    let reasoning = choices(true)?;
    let mut text = format!("Routing endpoints for {model} (OpenRouter):\n");
    let rows = listing
        .list
        .iter()
        .map(|endpoint| {
            let choice = Routing::Endpoint(endpoint.tag.clone());
            let ordinary = ordinary
                .iter()
                .find(|entry| entry.routing == choice)
                .expect("listed endpoint has a routing entry");
            let reasoning = reasoning
                .iter()
                .find(|entry| entry.routing == choice)
                .expect("listed endpoint has a reasoning entry");
            let compatible = ordinary.problem.is_none();
            let reasoning_compatible = reasoning.problem.is_none();
            text.push_str(&format!(
                "{}  tools {} · reasoning {} · {}\n",
                endpoint.tag,
                if compatible { "yes" } else { "no" },
                if reasoning_compatible { "yes" } else { "no" },
                ordinary.description
            ));
            Value::object([
                ("id", Value::string(&endpoint.tag)),
                ("provider", Value::optional(endpoint.provider.clone())),
                ("compatible", Value::Bool(compatible)),
                ("reasoning_compatible", Value::Bool(reasoning_compatible)),
                ("problem", Value::optional(ordinary.problem.clone())),
                (
                    "reasoning_problem",
                    Value::optional(reasoning.problem.clone()),
                ),
                ("description", Value::string(&ordinary.description)),
            ])
        })
        .collect();
    text.push_str("Use --routing auto, or --routing ENDPOINT_ID for a compatible endpoint.");
    let report = Report::new(
        Value::object([
            ("provider", Value::string("openrouter")),
            ("model", Value::string(model)),
            ("auto", Value::string("auto")),
            ("endpoints", Value::Array(rows)),
        ]),
        text,
    );
    let redactor = services.redactor();
    Ok(Report::new(
        redactor.value(&report.value),
        redactor.text(&report.text),
    ))
}

fn prices(model: &Model) -> String {
    match (model.prompt_price, model.completion_price) {
        (Some(input), Some(output)) => format!("${input:.2}/M in · ${output:.2}/M out"),
        _ => "prices not listed".into(),
    }
}

fn value(model: &Model, default: &Option<String>) -> Value {
    let price = |price: Option<f64>| price.map_or(Value::Null, Value::number);
    Value::object([
        ("id", Value::string(&model.id)),
        ("name", Value::string(&model.name)),
        (
            "efforts",
            Value::Array(
                model
                    .efforts
                    .iter()
                    .map(|effort| Value::string(effort.name()))
                    .collect(),
            ),
        ),
        ("input_price_per_million", price(model.prompt_price)),
        ("output_price_per_million", price(model.completion_price)),
        ("default", Value::Bool(Some(&model.id) == default.as_ref())),
    ])
}
