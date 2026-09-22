//! Small, editable user configuration. CLI options override defaults for one run.
use super::Store;
use crate::{
    json::{self, Value},
    session::Model,
    workspace::Access,
};
use std::{
    io,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Settings {
    pub model: Model,
    pub reduced_motion: bool,
    pub context_limit_bytes: usize,
    pub file_access: Access,
}
#[derive(Clone, Copy)]
pub enum Change {
    Model(Model),
    ToggleMotion,
    ToggleAccess,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            model: Model::Luna,
            reduced_motion: false,
            context_limit_bytes: 512 * 1024,
            file_access: Access::Local,
        }
    }
}
impl Settings {
    /// Update one preference under the same lock as readers. Other defaults are retained.
    pub fn update(store: &Store, change: Change) -> io::Result<Self> {
        let _lock = store.lock(
            "settings.lock",
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(2),
        )?;
        let mut settings = store
            .read("settings.json", 8192)?
            .map(|body| Self::parse(&body))
            .transpose()?
            .unwrap_or_default();
        match change {
            Change::Model(model) => settings.model = model,
            Change::ToggleMotion => settings.reduced_motion = !settings.reduced_motion,
            Change::ToggleAccess => {
                settings.file_access = match settings.file_access {
                    Access::Local => Access::Workspace,
                    Access::Workspace => Access::Local,
                }
            }
        }
        let body = format!(
            "{{\n  \"version\": 1,\n  \"model\": \"{}\",\n  \"reduced_motion\": {},\n  \"context_limit_bytes\": {},\n  \"file_access\": \"{}\"\n}}\n",
            settings.model.id(),
            settings.reduced_motion,
            settings.context_limit_bytes,
            settings.file_access.name()
        );
        store.replace("settings.json", &body)?;
        Ok(settings)
    }
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
            store.replace("settings.json", "{\n  \"version\": 1,\n  \"model\": \"gpt-5.6-luna\",\n  \"reduced_motion\": false,\n  \"context_limit_bytes\": 524288,\n  \"file_access\": \"local\"\n}\n")?;
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
                "version" | "model" | "reduced_motion" | "context_limit_bytes" | "file_access"
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
        let file_access = match value.get("file_access") {
            None => Access::Local,
            Some(value) => value.text().and_then(Access::parse).ok_or_else(invalid)?,
        };
        Ok(Self {
            model,
            reduced_motion,
            context_limit_bytes,
            file_access,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(any(windows, target_os = "linux"))]
    fn updates_keep_other_preferences_and_reject_damaged_files() {
        let fixture = crate::state::tests::Fixture::new();
        let Some(store) = fixture.store() else { return };
        store.replace("settings.json", r#"{"version":1,"model":"gpt-5.6-luna","reduced_motion":false,"context_limit_bytes":131072,"file_access":"workspace"}"#).unwrap();
        Settings::update(&store, Change::ToggleMotion).unwrap();
        Settings::update(&store, Change::Model(Model::Terra)).unwrap();
        let settings = Settings::load(&store).unwrap();
        assert_eq!(settings.model, Model::Terra);
        assert!(settings.reduced_motion);
        assert_eq!(settings.file_access, Access::Workspace);
        assert_eq!(settings.context_limit_bytes, 131072);
        store.replace("settings.json", "damaged fixture").unwrap();
        assert!(Settings::update(&store, Change::ToggleAccess).is_err());
        assert_eq!(
            store.read("settings.json", 8192).unwrap().unwrap(),
            "damaged fixture"
        );
    }
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
        assert_eq!(settings.file_access, Access::Local);
        let bounded = r#"{"version":1,"model":"gpt-5.6-luna","reduced_motion":false,"context_limit_bytes":131072,"file_access":"workspace"}"#;
        assert_eq!(
            Settings::parse(bounded).unwrap().file_access,
            Access::Workspace
        );
        assert!(Settings::parse(&bounded.replace("workspace", "unknown")).is_err());
    }
}
