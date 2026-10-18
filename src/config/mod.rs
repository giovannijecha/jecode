mod storage;

pub use storage::Store;

use crate::effort::Effort;
use crate::json::Value;
use crate::openrouter::{validate_key, validate_model};
use std::path::PathBuf;

// Deliberately no Debug implementation: settings contain a plaintext credential.
#[derive(Clone)]
pub struct Settings {
    pub api_key: String,
    pub model: String,
    pub effort: Effort,
    /// `context.auto_compact_percent`: applies to every conversation, not only new ones.
    pub auto_compact: u8,
}

impl Settings {
    pub fn new(api_key: String, model: String) -> Result<Self, String> {
        let api_key = api_key.trim().to_string();
        let model = model.trim().to_string();
        validate_key(&api_key)?;
        validate_model(&model)?;
        Ok(Self {
            api_key,
            model,
            effort: Effort::Default,
            auto_compact: crate::context::AUTO_COMPACT_DEFAULT,
        })
    }

    pub(super) fn parse(document: &Value) -> Result<Self, String> {
        let provider = document
            .get("openrouter")
            .ok_or("Missing openrouter configuration")?;
        let api_key = provider
            .get("api_key")
            .and_then(Value::as_str)
            .ok_or("Missing openrouter.api_key")?;
        let model = provider
            .get("model")
            .and_then(Value::as_str)
            .ok_or("Missing openrouter.model")?;
        let mut settings = Self::new(api_key.into(), model.into())?;
        if let Some(value) = provider.get("effort") {
            settings.effort = Effort::parse(value.as_str().ok_or("Invalid openrouter.effort")?)?;
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
