use super::{Settings, directory};
use crate::json::{self, Value};
use std::fs::{self, File, OpenOptions};
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
        super::validate_key(&settings.api_key)?;
        super::validate_model(&settings.model)?;
        let existing = self.document()?;
        // A malformed existing file is never replaced by setup or a model change.
        if let Some(document) = &existing {
            Settings::parse(document).map_err(|error| self.invalid(&error))?;
        }
        let mut document = existing.unwrap_or_else(|| Value::object([]));
        let Value::Object(root) = &mut document else {
            unreachable!()
        };
        let provider = root
            .entry("openrouter".into())
            .or_insert_with(|| Value::object([]));
        let Value::Object(provider) = provider else {
            return Err(self.invalid("openrouter must be an object"));
        };
        provider.insert("api_key".into(), Value::string(&settings.api_key));
        provider.insert("model".into(), Value::string(&settings.model));
        provider.insert("effort".into(), Value::string(settings.effort.name()));
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
        if !parent.exists() {
            fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
            }
        }
        let temporary = parent.join(format!(
            ".config-{}-{}.tmp",
            std::process::id(),
            NEXT_SAVE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
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
