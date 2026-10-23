mod storage;

pub use storage::Store;

use crate::backend::Routing;
pub use crate::backend::{validate_id as validate_backend_id, validate_model};
use crate::effort::Effort;
use crate::json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

// Deliberately no Debug implementation: settings contain a plaintext credential.
#[derive(Clone)]
pub struct Settings {
    /// The saved backend for new conversations. A resumed conversation keeps its own selection.
    pub backend_id: String,
    pub credentials: BTreeMap<String, String>,
    pub api_key: String,
    pub model: String,
    pub effort: Effort,
    /// `context.auto_compact_percent`: applies to every conversation, not only new ones.
    pub auto_compact: u8,
    /// Optional routing for the selected backend, scoped by backend in the file.
    pub routing: BTreeMap<String, Routing>,
}

impl Settings {
    pub fn new(api_key: String, model: String) -> Result<Self, String> {
        Self::selected("openrouter", api_key, model)
    }

    pub fn selected(backend_id: &str, api_key: String, model: String) -> Result<Self, String> {
        let api_key = api_key.trim().to_string();
        let model = model.trim().to_string();
        validate_backend_id(backend_id)?;
        if backend_id == "openrouter" {
            validate_key(&api_key)?;
        }
        validate_model(&model)?;
        Ok(Self {
            backend_id: backend_id.into(),
            credentials: BTreeMap::from([(backend_id.into(), api_key.clone())]),
            api_key,
            model,
            effort: Effort::Default,
            auto_compact: crate::context::AUTO_COMPACT_DEFAULT,
            routing: BTreeMap::new(),
        })
    }

    /// The saved routing for `model`, or Auto, the recommended default.
    pub fn routing_for(&self, model: &str) -> Routing {
        self.routing.get(model).cloned().unwrap_or(Routing::Auto)
    }

    pub fn credential_for(&self, backend_id: &str) -> Option<&str> {
        if backend_id == self.backend_id && !self.api_key.is_empty() {
            Some(&self.api_key)
        } else {
            self.credentials.get(backend_id).map(String::as_str)
        }
    }

    pub fn select_backend(&mut self, backend_id: &str, key: String) -> Result<(), String> {
        validate_backend_id(backend_id)?;
        if self.backend_id != backend_id {
            self.routing.clear();
        }
        self.backend_id = backend_id.into();
        self.api_key = key.clone();
        if !key.is_empty() {
            self.credentials.insert(backend_id.into(), key);
        }
        Ok(())
    }

    pub(super) fn parse(document: &Value) -> Result<Self, String> {
        let selection = document.get("selection");
        if selection.is_some_and(|value| !matches!(value, Value::Object(_))) {
            return Err("selection must be an object".into());
        }
        if document
            .get("backends")
            .is_some_and(|value| !matches!(value, Value::Object(_)))
        {
            return Err("backends must be an object".into());
        }
        if selection
            .and_then(|value| value.get("backend"))
            .is_some_and(|value| value.as_str().is_none())
        {
            return Err("selection.backend must be a string".into());
        }
        if selection
            .and_then(|value| value.get("model"))
            .is_some_and(|value| value.as_str().is_none())
        {
            return Err("selection.model must be a string".into());
        }
        let backend_id = selection
            .and_then(|value| value.get("backend"))
            .and_then(Value::as_str)
            .unwrap_or("openrouter");
        validate_backend_id(backend_id)?;
        let legacy = document.get("openrouter");
        if legacy
            .and_then(|value| value.get("routing"))
            .is_some_and(|value| !matches!(value, Value::Object(_)))
        {
            return Err("openrouter.routing must be an object".into());
        }
        let provider = document
            .get("backends")
            .and_then(|value| value.get(backend_id))
            .or_else(|| (backend_id == "openrouter").then_some(legacy).flatten());
        if provider.is_some_and(|value| !matches!(value, Value::Object(_))) {
            return Err(format!("backends.{backend_id} must be an object"));
        }
        if provider
            .and_then(|value| value.get("api_key"))
            .is_some_and(|value| value.as_str().is_none())
        {
            return Err(format!("backends.{backend_id}.api_key must be a string"));
        }
        let credentials: BTreeMap<String, String> = document
            .get("backends")
            .and_then(|value| match value {
                Value::Object(entries) => Some(entries),
                _ => None,
            })
            .into_iter()
            .flat_map(|entries| entries.iter())
            .filter_map(|(id, settings)| {
                settings
                    .get("api_key")
                    .and_then(Value::as_str)
                    .map(|key| (id.clone(), key.into()))
            })
            .collect();
        let api_key = provider
            .and_then(|value| value.get("api_key"))
            .and_then(Value::as_str)
            .or_else(|| {
                (backend_id == "openrouter")
                    .then_some(legacy)
                    .flatten()
                    .and_then(|value| value.get("api_key"))
                    .and_then(Value::as_str)
            })
            .unwrap_or("");
        if backend_id == "openrouter" {
            validate_key(api_key)?;
        }
        let model = selection
            .and_then(|value| value.get("model"))
            .and_then(Value::as_str)
            .or_else(|| {
                legacy
                    .and_then(|value| value.get("model"))
                    .and_then(Value::as_str)
            })
            .ok_or("Missing selected model")?;
        validate_model(model)?;
        let mut settings = Self {
            backend_id: backend_id.into(),
            credentials,
            api_key: api_key.into(),
            model: model.into(),
            effort: Effort::Default,
            auto_compact: crate::context::AUTO_COMPACT_DEFAULT,
            routing: BTreeMap::new(),
        };
        if !api_key.is_empty() {
            settings
                .credentials
                .insert(backend_id.into(), api_key.into());
        }
        if let Some(key) = legacy
            .and_then(|value| value.get("api_key"))
            .and_then(Value::as_str)
        {
            settings
                .credentials
                .entry("openrouter".into())
                .or_insert_with(|| key.into());
        }
        if let Some(value) = selection
            .and_then(|value| value.get("effort"))
            .or_else(|| legacy.and_then(|value| value.get("effort")))
        {
            settings.effort = Effort::parse(value.as_str().ok_or("Invalid selected effort")?)?;
        }
        // Absent in older files. Entries this version cannot use are skipped
        // here and kept in the file on save.
        let routing = provider.and_then(|value| value.get("routing")).or_else(|| {
            (backend_id == "openrouter")
                .then_some(legacy)
                .flatten()
                .and_then(|value| value.get("routing"))
        });
        if let Some(routing) = routing {
            let Value::Object(entries) = routing else {
                return Err(if backend_id == "openrouter" {
                    "openrouter.routing must be an object".into()
                } else {
                    format!("backends.{backend_id}.routing must be an object")
                });
            };
            settings.routing = entries
                .iter()
                .filter(|(model, _)| validate_model(model).is_ok())
                .filter_map(|(model, value)| {
                    Routing::parse(value)
                        .filter(|routing| *routing != Routing::Default)
                        .map(|routing| (model.clone(), routing))
                })
                .collect();
        }
        // Configurations written before this preference load with the default.
        if let Some(context) = document.get("context") {
            if !matches!(context, Value::Object(_)) {
                return Err("context must be an object".into());
            }
            if let Some(value) = context.get("auto_compact_percent") {
                settings.auto_compact = value
                    .as_usize()
                    .ok_or_else(|| "Auto-compact must be a whole percentage from 1 to 99".into())
                    .and_then(crate::context::auto_compact_percent)
                    .map_err(|error| format!("Invalid context.auto_compact_percent: {error}"))?;
            }
        }
        Ok(settings)
    }
}

pub fn validate_key(key: &str) -> Result<(), String> {
    crate::backend::validate_credential("openrouter", key)
}

pub fn directory(
    override_path: Option<PathBuf>,
    user_home: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let path = match override_path {
        Some(path) => path,
        None => user_home.ok_or("Could not locate your home directory; set JECODE_HOME to an absolute configuration directory")?.join(".jecode"),
    };
    if !path.is_absolute() {
        return Err("The Jecode configuration directory must be an absolute path".into());
    }
    Ok(path)
}

pub fn environment_settings(
    key: Option<String>,
    model: Option<String>,
) -> Result<Option<Settings>, String> {
    match (key, model) {
        (Some(key), Some(model)) => Settings::new(key, model).map(Some),
        (None, None) => Ok(None),
        _ => Err("Both OPENROUTER_API_KEY and OPENROUTER_MODEL are needed for environment-only configuration; run jecode setup instead".into()),
    }
}

#[cfg(test)]
mod tests;
