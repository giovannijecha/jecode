use super::{Api, validate_model};
use crate::effort::Effort;
use crate::json::Value;

#[derive(Clone)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub prompt_price: Option<f64>,
    pub completion_price: Option<f64>,
    pub efforts: Vec<Effort>,
}

impl Api {
    pub fn models(&self, query: &str) -> Result<Vec<Model>, String> {
        let path = format!(
            "/models?supported_parameters=tools&q={}&limit=10",
            percent_encode(query)
        );
        let value = self.request("GET", &path, None)?;
        parse_models(value, 10)
    }

    pub fn catalog(&self) -> Result<Vec<Model>, String> {
        parse_models(
            self.request("GET", "/models?supported_parameters=tools", None)?,
            usize::MAX,
        )
    }
}

fn parse_models(value: Value, limit: usize) -> Result<Vec<Model>, String> {
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or("OpenRouter returned an invalid model catalog")?;
    let mut models = Vec::new();
    for model in data {
        let Some(id) = model.get("id").and_then(Value::as_str) else {
            continue;
        };
        if validate_model(id).is_err() || models.iter().any(|model: &Model| model.id == id) {
            continue;
        }
        let supports_tools = model
            .get("supported_parameters")
            .and_then(Value::as_array)
            .is_some_and(|parameters| {
                parameters
                    .iter()
                    .any(|parameter| parameter.as_str() == Some("tools"))
            });
        if !supports_tools {
            continue;
        }
        let name = model.get("name").and_then(Value::as_str).unwrap_or(id);
        let pricing = model.get("pricing");
        models.push(Model {
            id: id.into(),
            name: name.into(),
            prompt_price: pricing.and_then(|pricing| price(pricing.get("prompt"))),
            completion_price: pricing.and_then(|pricing| price(pricing.get("completion"))),
            efforts: efforts(model),
        });
        if models.len() == limit {
            break;
        }
    }
    Ok(models)
}

fn efforts(model: &Value) -> Vec<Effort> {
    let mut result = vec![Effort::Default];
    let Some(reasoning) = model.get("reasoning") else {
        return result;
    };
    let mandatory = reasoning.get("mandatory") == Some(&Value::Bool(true));
    match reasoning.get("supported_efforts") {
        Some(Value::Array(values)) => {
            for value in values {
                if let Some(value) = value.as_str()
                    && let Ok(effort) = Effort::parse(value)
                    && !(result.contains(&effort) || mandatory && effort == Effort::None)
                {
                    result.push(effort);
                }
            }
        }
        Some(Value::Null) => result.extend(
            Effort::LEVELS
                .into_iter()
                .filter(|effort| !mandatory || *effort != Effort::None),
        ),
        _ => {}
    }
    result
}

fn price(value: Option<&Value>) -> Option<f64> {
    let value = value?.as_str()?.parse::<f64>().ok()? * 1_000_000.0;
    (value.is_finite() && value >= 0.0).then_some(value)
}

pub(super) fn percent_encode(text: &str) -> String {
    use std::fmt::Write;
    let mut encoded = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            write!(encoded, "%{byte:02X}").expect("writing to a string");
        }
    }
    encoded
}

#[cfg(test)]
mod tests;
