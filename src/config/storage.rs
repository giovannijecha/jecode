use super::{Settings, directory};
use crate::json::{self, Value};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const CONFIG_LIMIT: usize = 64 * 1024;
static NEXT_SAVE: AtomicU64 = AtomicU64::new(0);

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn discover() -> Result<Self, String> {
        let home_variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let path = directory(
            std::env::var_os("JECODE_HOME").map(PathBuf::from),
            std::env::var_os(home_variable).map(PathBuf::from),
        )?;
        Ok(Self::new(path))
    }

    pub fn new(directory: PathBuf) -> Self {
        Self {
            path: directory.join("config.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Restore a provider's own model instead of carrying a model across providers.
    pub fn previous_model(&self, backend: &str) -> Result<Option<String>, String> {
        Ok(self.document()?.and_then(|document| {
            let scoped = document
                .get("backends")
                .and_then(|backends| backends.get(backend));
            let legacy = (backend == "openrouter")
                .then(|| document.get("openrouter"))
                .flatten();
            legacy
                .and_then(|provider| provider.get("model"))
                .and_then(Value::as_str)
                .or_else(|| scoped?.get("model")?.as_str())
                .filter(|model| super::validate_model(model).is_ok())
                .map(str::to_owned)
        }))
    }

    pub fn load(&self) -> Result<Option<Settings>, String> {
        self.document()?
            .map(|document| Settings::parse(&document).map_err(|error| self.invalid(&error)))
            .transpose()
    }

    fn document(&self) -> Result<Option<Value>, String> {
        let metadata = match fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(self.invalid(&format!("Could not access configuration: {error}")));
            }
        };
        if !metadata.is_file() || metadata.len() > CONFIG_LIMIT as u64 {
            return Err(self.invalid("Configuration must be a regular file no larger than 64 KiB"));
        }
        let mut text = String::new();
        File::open(&self.path)
            .and_then(|file| file.take(CONFIG_LIMIT as u64 + 1).read_to_string(&mut text))
            .map_err(|error| self.invalid(&format!("Could not read configuration: {error}")))?;
        if text.len() > CONFIG_LIMIT {
            return Err(self.invalid("Configuration exceeds 64 KiB"));
        }
        let document = json::parse(&text).map_err(|error| self.invalid(&error))?;
        if !matches!(document, Value::Object(_)) {
            return Err(self.invalid("Configuration must be a JSON object"));
        }
        Ok(Some(document))
    }

    pub fn save(&self, settings: &Settings) -> Result<(), String> {
        if !settings.has_model() {
            return Err(
                "Choose a default model in Settings > Default model before saving configuration"
                    .into(),
            );
        }
        super::validate_backend_id(&settings.backend_id)?;
        if settings.backend_id == "openrouter" && !settings.api_key.is_empty() {
            super::validate_key(&settings.api_key)?;
        }
        super::validate_model(&settings.model)?;
        crate::context::auto_compact_percent(settings.auto_compact.into())?;
        let existing = self.document()?;
        // A malformed existing file is never replaced by setup or a model change.
        if let Some(document) = &existing {
            Settings::parse(document).map_err(|error| self.invalid(&error))?;
        }
        let mut document = existing.unwrap_or_else(|| Value::object([]));
        let Value::Object(root) = &mut document else {
            unreachable!()
        };
        let selection = root
            .entry("selection".into())
            .or_insert_with(|| Value::object([]));
        let Value::Object(selection) = selection else {
            return Err(self.invalid("selection must be an object"));
        };
        selection.insert("backend".into(), Value::string(&settings.backend_id));
        selection.insert("model".into(), Value::string(&settings.model));
        selection.insert("effort".into(), Value::string(settings.effort.name()));
        let backends = root
            .entry("backends".into())
            .or_insert_with(|| Value::object([]));
        let Value::Object(backends) = backends else {
            return Err(self.invalid("backends must be an object"));
        };
        for (id, key) in settings
            .credentials
            .iter()
            .chain(std::iter::once((&settings.backend_id, &settings.api_key)))
        {
            if key.is_empty() {
                continue;
            }
            super::validate_backend_id(id)?;
            let provider = backends
                .entry(id.clone())
                .or_insert_with(|| Value::object([]));
            let Value::Object(provider) = provider else {
                return Err(self.invalid(&format!("backends.{id} must be an object")));
            };
            provider.insert("api_key".into(), Value::string(key));
        }
        let provider = backends
            .entry(settings.backend_id.clone())
            .or_insert_with(|| Value::object([]));
        let Value::Object(provider) = provider else {
            return Err(self.invalid("Selected backend must be an object"));
        };
        provider.insert("model".into(), Value::string(&settings.model));
        if !settings.routing.is_empty() {
            let provider = backends
                .entry(settings.backend_id.clone())
                .or_insert_with(|| Value::object([]));
            let Value::Object(provider) = provider else {
                return Err(self.invalid(&format!(
                    "backends.{} must be an object",
                    settings.backend_id
                )));
            };
            let routing = provider
                .entry("routing".into())
                .or_insert_with(|| Value::object([]));
            let Value::Object(routing) = routing else {
                return Err(self.invalid(&format!(
                    "backends.{}.routing must be an object",
                    settings.backend_id
                )));
            };
            for (model, preference) in &settings.routing {
                routing.insert(model.clone(), preference.value());
            }
        }
        // Maintain the original OpenRouter object for installed versions and
        // preserve every unrelated field when changing the selected backend.
        if settings.backend_id == "openrouter" || root.contains_key("openrouter") {
            let provider = root
                .entry("openrouter".into())
                .or_insert_with(|| Value::object([]));
            let Value::Object(provider) = provider else {
                return Err(self.invalid("openrouter must be an object"));
            };
            if let Some(key) = settings
                .credential_for("openrouter")
                .filter(|key| !key.is_empty())
            {
                provider.insert("api_key".into(), Value::string(key));
            } else {
                provider.remove("api_key");
            }
            if settings.backend_id == "openrouter" {
                provider.insert("model".into(), Value::string(&settings.model));
                provider.insert("effort".into(), Value::string(settings.effort.name()));
            }
            if settings.backend_id == "openrouter" && !settings.routing.is_empty() {
                let routing = provider
                    .entry("routing".into())
                    .or_insert_with(|| Value::object([]));
                let Value::Object(routing) = routing else {
                    return Err(self.invalid("openrouter.routing must be an object"));
                };
                for (model, preference) in &settings.routing {
                    routing.insert(model.clone(), preference.value());
                }
            }
        }
        // Kept beside, not inside, the credentials and model object.
        let context = root
            .entry("context".into())
            .or_insert_with(|| Value::object([]));
        let Value::Object(context) = context else {
            return Err(self.invalid("context must be an object"));
        };
        context.insert(
            "auto_compact_percent".into(),
            Value::number(settings.auto_compact),
        );
        let permissions = root
            .entry("permissions".into())
            .or_insert_with(|| Value::object([]));
        let Value::Object(permissions) = permissions else {
            return Err(self.invalid("permissions must be an object"));
        };
        permissions.insert(
            "default".into(),
            Value::string(settings.default_permissions.id()),
        );
        let encoded = format!("{}\n", document.pretty());
        if encoded.len() > CONFIG_LIMIT {
            return Err(self.invalid("Configuration exceeds 64 KiB"));
        }
        self.atomic_save(encoded.as_bytes())
            .map_err(|error| format!("Could not save {}: {error}", self.path.display()))
    }

    fn invalid(&self, error: &str) -> String {
        format!(
            "{}: {error}. The file was left unchanged; correct it before running jecode setup.",
            self.path.display()
        )
    }

    fn atomic_save(&self, bytes: &[u8]) -> std::io::Result<()> {
        if self.path.exists() && fs::metadata(&self.path)?.permissions().readonly() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Configuration is read-only",
            ));
        }
        let parent = self.path.parent().expect("configuration parent");
        crate::disk::private_directory(parent)?;
        let temporary = parent.join(format!(
            ".config-{}-{}.tmp",
            std::process::id(),
            NEXT_SAVE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = crate::disk::private_options()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}
