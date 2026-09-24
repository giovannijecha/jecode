//! Small, editable user configuration. CLI options override defaults for one run.
use super::Store;
use crate::{
    json::{self, Value},
    session::Model,
    workspace::Access,
};
use std::{
    collections::BTreeMap,
    fmt, io,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Settings {
    pub model: Model,
    pub reduced_motion: bool,
    pub context_limit_bytes: usize,
    pub file_access: Access,
    pub windows_powershell_executable: Option<String>,
    extra: BTreeMap<String, Value>,
}
#[derive(Debug)]
pub struct ShellConfigError(String);
impl fmt::Display for ShellConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl std::error::Error for ShellConfigError {}
pub(crate) fn shell_config_error(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        ShellConfigError(message.into()),
    )
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
            windows_powershell_executable: None,
            extra: BTreeMap::new(),
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
        let mut fields = settings.extra.clone();
        fields.insert("version".into(), Value::Number("1".into()));
        fields.insert("model".into(), Value::String(settings.model.id().into()));
        fields.insert(
            "effort".into(),
            settings
                .model
                .effort()
                .map_or(Value::Null, |effort| Value::String(effort.into())),
        );
        fields.insert(
            "reduced_motion".into(),
            Value::Bool(settings.reduced_motion),
        );
        fields.insert(
            "context_limit_bytes".into(),
            Value::Number(settings.context_limit_bytes.to_string()),
        );
        fields.insert(
            "file_access".into(),
            Value::String(settings.file_access.name().into()),
        );
        if let Some(executable) = &settings.windows_powershell_executable {
            fields.insert(
                "windows_powershell_executable".into(),
                Value::String(executable.clone()),
            );
        } else {
            fields.remove("windows_powershell_executable");
        }
        let body = json::encode(&Value::Object(fields), 8192)
            .map_err(|_| io::Error::other("settings exceed size limit"))?;
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
            store.replace("settings.json", "{\n  \"version\": 1,\n  \"model\": \"gpt-5.6-luna\",\n  \"effort\": \"medium\",\n  \"reduced_motion\": false,\n  \"context_limit_bytes\": 524288,\n  \"file_access\": \"local\"\n}\n")?;
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
        if value.get("version").and_then(Value::unsigned) != Some(1) {
            return Err(invalid());
        }
        let effort = match value.get("effort") {
            None => Some("medium"), // historical v1 effective request
            Some(Value::Null) => None,
            Some(Value::String(effort)) => Some(effort.as_str()),
            _ => return Err(invalid()),
        };
        let model = value
            .get("model")
            .and_then(Value::text)
            .and_then(|id| Model::new(id, effort))
            .ok_or_else(invalid)?;
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
        let windows_powershell_executable = match value.get("windows_powershell_executable") {
            None | Some(Value::Null) => None,
            Some(Value::String(path)) if !path.is_empty() => Some(path.clone()),
            _ => {
                return Err(shell_config_error(
                    "windows_powershell_executable in ~/.jecode/v1/settings.json must be a nonempty absolute path to pwsh.exe, or null",
                ));
            }
        };
        Ok(Self {
            model,
            reduced_motion,
            context_limit_bytes,
            file_access,
            windows_powershell_executable,
            extra: fields
                .iter()
                .filter(|(key, _)| {
                    !matches!(
                        key.as_str(),
                        "version"
                            | "model"
                            | "effort"
                            | "reduced_motion"
                            | "context_limit_bytes"
                            | "file_access"
                            | "windows_powershell_executable"
                    )
                })
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
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
    #[test]
    #[cfg(any(windows, target_os = "linux"))]
    fn legacy_effort_and_unrelated_fields_survive_explicit_update() {
        let fixture = crate::state::tests::Fixture::new();
        let Some(store) = fixture.store() else { return };
        let legacy = r#"{"version":1,"model":"future-model","reduced_motion":false,"context_limit_bytes":131072,"file_access":"local","future_setting":{"enabled":true}}"#;
        store.replace("settings.json", legacy).unwrap();
        let settings = Settings::load(&store).unwrap();
        assert_eq!(settings.model.id(), "future-model");
        assert_eq!(settings.model.effort(), Some("medium"));
        assert_eq!(store.read("settings.json", 8192).unwrap().unwrap(), legacy);
        Settings::update(&store, Change::ToggleMotion).unwrap();
        let updated = store.read("settings.json", 8192).unwrap().unwrap();
        let value = json::parse(&updated, Default::default()).unwrap();
        assert_eq!(
            value
                .get("future_setting")
                .and_then(|value| value.get("enabled")),
            Some(&Value::Bool(true))
        );
        assert_eq!(value.get("effort").and_then(Value::text), Some("medium"));
        Settings::update(
            &store,
            Change::Model(Model::new("another-model", None).unwrap()),
        )
        .unwrap();
        assert_eq!(Settings::load(&store).unwrap().model.effort(), None);
    }

    #[test]
    #[cfg(any(windows, target_os = "linux"))]
    fn windows_shell_selection_is_user_scoped_and_survives_other_updates() {
        let fixture = crate::state::tests::Fixture::new();
        let Some(store) = fixture.store() else { return };
        store.replace("settings.json", r#"{"version":1,"model":"gpt-5.6-luna","reduced_motion":false,"context_limit_bytes":131072,"windows_powershell_executable":"C:\\Program Files\\PowerShell\\7\\pwsh.exe"}"#).unwrap();
        let settings = Settings::load(&store).unwrap();
        assert_eq!(
            settings.windows_powershell_executable.as_deref(),
            Some(r"C:\Program Files\PowerShell\7\pwsh.exe")
        );
        Settings::update(&store, Change::ToggleMotion).unwrap();
        assert_eq!(
            Settings::load(&store)
                .unwrap()
                .windows_powershell_executable,
            settings.windows_powershell_executable
        );
        let body = store.read("settings.json", 8192).unwrap().unwrap();
        let Value::Object(mut fields) = json::parse(&body, Default::default()).unwrap() else {
            panic!("settings must be an object");
        };
        fields.insert(
            "windows_powershell_executable".into(),
            Value::Number("42".into()),
        );
        let invalid = json::encode(&Value::Object(fields), 8192).unwrap();
        let error = Settings::parse(&invalid).err().unwrap();
        assert!(error.to_string().contains("windows_powershell_executable"));
    }
}
