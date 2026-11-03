use super::{CheckFailure, CheckObservation, Profile};
use crate::{
    disk,
    json::{self, Value},
};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

const LIMIT: usize = 256 * 1024;

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
    lock: PathBuf,
}

impl Store {
    pub fn new(home: PathBuf) -> Self {
        Self {
            path: home.join("lsp.json"),
            lock: home.join("lsp.lock"),
        }
    }

    pub fn load(&self) -> Result<Vec<Profile>, String> {
        let Some(metadata) = disk::metadata_if_present(&self.path).map_err(|e| e.to_string())?
        else {
            return Ok(Vec::new());
        };
        if !metadata.is_file() || disk::linked(&metadata) || metadata.len() > LIMIT as u64 {
            return Err("LSP configuration must be a regular file under 256 KiB".into());
        }
        let mut bytes = Vec::new();
        File::open(&self.path)
            .map_err(|e| e.to_string())?
            .take(LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > LIMIT {
            return Err("LSP configuration exceeds 256 KiB".into());
        }
        let root = json::parse(
            std::str::from_utf8(&bytes).map_err(|_| "LSP configuration is not UTF-8")?,
        )?;
        let entries = root
            .get("servers")
            .and_then(Value::as_array)
            .ok_or("LSP configuration must contain a servers array")?;
        let profiles = entries.iter().map(decode).collect::<Result<Vec<_>, _>>()?;
        let mut seen = std::collections::HashSet::new();
        if profiles.iter().any(|p| !seen.insert(&p.id)) {
            return Err("Duplicate LSP server ID".into());
        }
        Ok(profiles)
    }

    pub fn save(&self, mut profile: Profile) -> Result<(), String> {
        profile.validate()?;
        self.mutate(|profiles| {
            if profile
                .last_check
                .as_ref()
                .is_some_and(|check| check.fingerprint != profile.analysis_fingerprint())
            {
                profile.last_check = None;
            }
            if profile
                .last_check_failure
                .as_ref()
                .is_some_and(|check| check.fingerprint != profile.analysis_fingerprint())
            {
                profile.last_check_failure = None;
            }
            if let Some(old) = profiles.iter_mut().find(|p| p.id == profile.id) {
                *old = profile;
            } else {
                profiles.push(profile);
            }
            Ok(())
        })
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        self.mutate(|profiles| {
            profiles.retain(|p| p.id != id);
            Ok(())
        })
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        self.mutate(|profiles| {
            let profile = profiles
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or("Unknown LSP server")?;
            profile.enabled = enabled;
            Ok(())
        })
    }

    pub fn record_check(
        &self,
        id: &str,
        result: Result<CheckObservation, CheckFailure>,
    ) -> Result<bool, String> {
        let fingerprint = match &result {
            Ok(check) => check.fingerprint.clone(),
            Err(failure) => failure.fingerprint.clone(),
        };
        let mut recorded = false;
        self.mutate(|profiles| {
            let Some(profile) = profiles.iter_mut().find(|profile| profile.id == id) else {
                return Ok(());
            };
            if profile.analysis_fingerprint() != fingerprint {
                return Ok(());
            }
            match result {
                Ok(check) => {
                    profile.last_check = Some(check);
                    profile.last_check_failure = None;
                }
                Err(failure) => {
                    profile.last_check = None;
                    profile.last_check_failure = Some(failure);
                }
            }
            recorded = true;
            Ok(())
        })?;
        Ok(recorded)
    }

    fn mutate(
        &self,
        update: impl FnOnce(&mut Vec<Profile>) -> Result<(), String>,
    ) -> Result<(), String> {
        disk::ensure_private_directory(self.path.parent().expect("LSP home"))
            .map_err(|e| e.to_string())?;
        #[cfg(windows)]
        disk::protect_owner_only(self.path.parent().expect("LSP home"), true)?;
        let _guard = disk::lock::Exclusive::acquire(&self.lock).map_err(|e| e.to_string())?;
        let mut profiles = self.load()?;
        update(&mut profiles)?;
        let document = Value::object([(
            "servers",
            Value::Array(profiles.iter().map(encode).collect()),
        )]);
        let bytes = document.pretty().into_bytes();
        if bytes.len() > LIMIT {
            return Err("LSP configuration exceeds 256 KiB".into());
        }
        disk::atomic_write(&self.path, &bytes).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        disk::protect_owner_only(&self.path, false)?;
        Ok(())
    }
}

fn strings(value: &Value, key: &str) -> Result<Vec<String>, String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Missing LSP {key}"))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("Invalid LSP {key}"))
        })
        .collect()
}
fn string(value: &Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("Missing LSP {key}"))
}
fn path(value: &Value) -> Option<PathBuf> {
    value.as_str().map(PathBuf::from)
}
fn array(values: &[String]) -> Value {
    Value::Array(values.iter().map(Value::string).collect())
}

fn decode(value: &Value) -> Result<Profile, String> {
    let environment = match value.get("environment") {
        Some(Value::Object(entries)) => entries
            .iter()
            .map(|(key, value)| {
                value
                    .as_str()
                    .map(|v| (key.clone(), v.to_owned()))
                    .ok_or("Invalid LSP environment")
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?,
        _ => return Err("Missing LSP environment".into()),
    };
    let extensions = match value.get("extensions") {
        Some(Value::Object(entries)) => entries
            .iter()
            .map(|(key, value)| {
                value
                    .as_str()
                    .map(|v| (key.clone(), v.to_owned()))
                    .ok_or("Invalid LSP extension mapping")
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?,
        _ => return Err("Missing LSP extensions".into()),
    };
    let last_check = value.get("last_check").and_then(|v| {
        Some(CheckObservation {
            capabilities: v.get("capabilities")?.clone(),
            root: path(v.get("root")?)?,
            executable: v.get("executable")?.as_str()?.to_owned(),
            fingerprint: v.get("fingerprint")?.as_str()?.to_owned(),
        })
    });
    let last_check_failure = value.get("last_check_failure").and_then(|v| {
        Some(CheckFailure {
            detail: v.get("detail")?.as_str()?.to_owned(),
            fingerprint: v.get("fingerprint")?.as_str()?.to_owned(),
        })
    });
    let profile = Profile {
        id: string(value, "id")?,
        name: string(value, "name")?,
        enabled: matches!(value.get("enabled"), Some(Value::Bool(true))),
        executable: string(value, "executable")?,
        arguments: strings(value, "arguments")?,
        environment,
        languages: strings(value, "languages")?,
        extensions,
        root_markers: strings(value, "root_markers")?,
        root_override: value.get("root_override").and_then(path),
        initialization_options: value
            .get("initialization_options")
            .cloned()
            .unwrap_or(Value::Null),
        settings: value.get("settings").cloned().unwrap_or(Value::object([])),
        request_timeout_ms: value
            .get("request_timeout_ms")
            .and_then(Value::as_usize)
            .unwrap_or(10_000) as u64,
        last_check,
        last_check_failure,
    };
    profile.validate()?;
    Ok(profile)
}

pub(super) fn encode(profile: &Profile) -> Value {
    let environment = Value::Object(
        profile
            .environment
            .iter()
            .map(|(k, v)| (k.clone(), Value::string(v)))
            .collect(),
    );
    let check = profile.last_check.as_ref().map_or(Value::Null, |check| {
        Value::object([
            ("capabilities", check.capabilities.clone()),
            ("root", Value::string(check.root.to_string_lossy())),
            ("executable", Value::string(&check.executable)),
            ("fingerprint", Value::string(&check.fingerprint)),
        ])
    });
    let check_failure = profile
        .last_check_failure
        .as_ref()
        .map_or(Value::Null, |failure| {
            Value::object([
                ("detail", Value::string(&failure.detail)),
                ("fingerprint", Value::string(&failure.fingerprint)),
            ])
        });
    Value::object([
        ("id", Value::string(&profile.id)),
        ("name", Value::string(&profile.name)),
        ("enabled", Value::Bool(profile.enabled)),
        ("executable", Value::string(&profile.executable)),
        ("arguments", array(&profile.arguments)),
        ("environment", environment),
        ("languages", array(&profile.languages)),
        (
            "extensions",
            Value::Object(
                profile
                    .extensions
                    .iter()
                    .map(|(k, v)| (k.clone(), Value::string(v)))
                    .collect(),
            ),
        ),
        ("root_markers", array(&profile.root_markers)),
        (
            "root_override",
            profile
                .root_override
                .as_ref()
                .map_or(Value::Null, |p| Value::string(p.to_string_lossy())),
        ),
        (
            "initialization_options",
            profile.initialization_options.clone(),
        ),
        ("settings", profile.settings.clone()),
        (
            "request_timeout_ms",
            Value::number(profile.request_timeout_ms),
        ),
        ("last_check", check),
        ("last_check_failure", check_failure),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;

    #[test]
    fn check_invalidated_by_environment_value_change() {
        let home = Directory::new();
        let store = Store::new(home.path().to_path_buf());
        let mut profile = Profile::draft_guided_rust();
        profile.last_check = Some(CheckObservation {
            capabilities: Value::object([]),
            root: home.path().to_path_buf(),
            executable: profile.executable.clone(),
            fingerprint: profile.analysis_fingerprint(),
        });
        store.save(profile.clone()).unwrap();
        profile.environment.insert("FLAG".into(), "new".into());
        store.save(profile).unwrap();
        assert!(store.load().unwrap()[0].last_check.is_none());
    }

    #[test]
    fn record_check_preserves_current_activation_and_rejects_stale_or_removed_entry() {
        let home = Directory::new();
        let store = Store::new(home.path().to_path_buf());
        let profile = Profile::draft_guided_rust();
        let id = profile.id.clone();
        let fingerprint = profile.analysis_fingerprint();
        store.save(profile).unwrap();
        store.set_enabled(&id, false).unwrap();
        assert!(
            store
                .record_check(
                    &id,
                    Err(CheckFailure {
                        detail: "Missing executable".into(),
                        fingerprint: fingerprint.clone()
                    })
                )
                .unwrap()
        );
        let saved = store.load().unwrap().remove(0);
        assert!(!saved.enabled);
        assert_eq!(
            saved.last_check_failure.as_ref().unwrap().detail,
            "Missing executable"
        );
        let mut changed = saved;
        changed.arguments.push("--changed".into());
        store.save(changed).unwrap();
        assert!(
            !store
                .record_check(
                    &id,
                    Ok(CheckObservation {
                        capabilities: Value::object([]),
                        root: home.path().to_path_buf(),
                        executable: "rust-analyzer".into(),
                        fingerprint: fingerprint.clone()
                    })
                )
                .unwrap()
        );
        store.remove(&id).unwrap();
        assert!(
            !store
                .record_check(
                    &id,
                    Err(CheckFailure {
                        detail: "late".into(),
                        fingerprint
                    })
                )
                .unwrap()
        );
        assert!(store.load().unwrap().is_empty());
    }
}
