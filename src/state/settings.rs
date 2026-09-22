//! Small, editable user configuration. CLI options override defaults for one run.
use super::Store;
use crate::{
    json::{self, Value},
    session::Model,
};
use std::{
    io,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

pub struct Settings {
    pub model: Model,
    pub reduced_motion: bool,
    pub context_limit_bytes: usize,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            model: Model::Luna,
            reduced_motion: false,
            context_limit_bytes: 512 * 1024,
        }
    }
}
impl Settings {
    pub fn user() -> io::Result<Self> {
        Self::load(&Store::user()?)
    }
    pub fn load(store: &Store) -> io::Result<Self> {
        let _lock = store.lock(
            "settings.lock",
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(2),
        )?;
        let Some(body) = store.read("settings.json", 8192)? else {
            store.replace("settings.json", "{\n  \"version\": 1,\n  \"model\": \"gpt-5.6-luna\",\n  \"reduced_motion\": false,\n  \"context_limit_bytes\": 524288\n}\n")?;
            return Ok(Self::default());
        };
        Self::parse(&body)
    }
    fn parse(body: &str) -> io::Result<Self> {
        let invalid = || io::Error::other("invalid ~/.jecode/v1/settings.json");
        let value = json::parse(
            body,
            json::Limits {
                bytes: 8192,
                nodes: 32,
                depth: 4,
            },
        )
        .map_err(|_| invalid())?;
        let Value::Object(fields) = &value else {
            return Err(invalid());
        };
        if fields.keys().any(|key| {
            !matches!(
                key.as_str(),
                "version" | "model" | "reduced_motion" | "context_limit_bytes"
            )
        }) || value.get("version").and_then(Value::unsigned) != Some(1)
        {
            return Err(invalid());
        }
        let model = match value.get("model").and_then(Value::text) {
            Some("gpt-5.6-luna") => Model::Luna,
            Some("gpt-5.6-terra") => Model::Terra,
            _ => return Err(invalid()),
        };
        let reduced_motion = match value.get("reduced_motion") {
            Some(Value::Bool(v)) => *v,
            _ => return Err(invalid()),
        };
        let context_limit_bytes = value
            .get("context_limit_bytes")
            .and_then(Value::unsigned)
            .filter(|n| (65536..=1572864).contains(n))
            .ok_or_else(invalid)? as usize;
        Ok(Self {
            model,
            reduced_motion,
            context_limit_bytes,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_or_unsupported_preferences_are_not_silently_ignored() {
        for body in [
            "{}",
            r#"{"version":9}"#,
            r#"{"version":1,"model":"unknown"}"#,
        ] {
            assert!(Settings::parse(body).is_err());
        }
        let settings = Settings::parse(r#"{"version":1,"model":"gpt-5.6-terra","reduced_motion":true,"context_limit_bytes":131072}"#).unwrap();
        assert_eq!(settings.model, Model::Terra);
        assert!(settings.reduced_motion);
        assert_eq!(settings.context_limit_bytes, 131072);
    }
}
