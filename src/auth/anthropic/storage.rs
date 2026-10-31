use crate::auth::openai::native;
use crate::cancel::Cancellation;
use crate::json::{self, Value};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const LIMIT: usize = 32 * 1024;

#[derive(Clone)]
pub(super) struct Record {
    pub id: String,
    pub label: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
    pub scopes: String,
}

impl Record {
    pub fn enabled(&self) -> bool {
        !self.access_token.is_empty()
            && !self.refresh_token.is_empty()
            && self
                .scopes
                .split_whitespace()
                .any(|scope| scope == "user:inference")
    }

    pub fn clear(&mut self) {
        self.access_token.clear();
        self.refresh_token.clear();
        self.expires_at = 0;
    }

    fn value(&self) -> Value {
        Value::object([
            ("format", Value::string("jecode-anthropic-account-v1")),
            ("id", Value::string(&self.id)),
            ("label", Value::string(&self.label)),
            ("access_token", Value::string(&self.access_token)),
            ("refresh_token", Value::string(&self.refresh_token)),
            ("expires_at", Value::number(self.expires_at)),
            ("scopes", Value::string(&self.scopes)),
        ])
    }

    fn parse(value: &Value) -> Result<Self, String> {
        let field = |name: &str| -> Result<String, String> {
            value
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("Invalid Claude account field {name}"))
        };
        if field("format")? != "jecode-anthropic-account-v1" {
            return Err("Unknown Claude account format".into());
        }
        let expires_at = match value.get("expires_at") {
            Some(Value::Number(raw)) => raw.parse().map_err(|_| "Invalid Claude token expiry")?,
            _ => return Err("Invalid Claude token expiry".into()),
        };
        let record = Self {
            id: field("id")?,
            label: field("label")?,
            access_token: field("access_token")?,
            refresh_token: field("refresh_token")?,
            expires_at,
            scopes: field("scopes")?,
        };
        if record.id.is_empty() || record.id.len() > 256 || record.label.len() > 1024 {
            return Err("Invalid Claude account identity".into());
        }
        Ok(record)
    }
}

pub(super) struct Files {
    directory: PathBuf,
}

impl Files {
    pub fn new(home: &Path) -> Self {
        Self {
            directory: home.join("accounts").join("anthropic"),
        }
    }

    fn ensure(&self) -> Result<(), String> {
        if let Some(parent) = self.directory.parent().filter(|path| path.exists()) {
            check_path(parent, true)?;
        }
        fs::create_dir_all(&self.directory)
            .map_err(|_| "Could not create Claude account directory")?;
        check_path(&self.directory, true)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.directory, fs::Permissions::from_mode(0o700))
                .map_err(|_| "Could not protect Claude account directory")?;
        }
        #[cfg(windows)]
        native::protect(&self.directory, true)?;
        Ok(())
    }

    pub fn lock(&self, cancel: &Cancellation) -> Result<File, String> {
        self.ensure()?;
        let path = self.directory.join(".lock");
        if path.exists() {
            check_path(&path, false)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|_| "Could not open Claude account lock")?;
        protect_file(&path)?;
        loop {
            if cancel.requested() {
                return Err("Operation cancelled".into());
            }
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(TryLockError::WouldBlock) => cancel.wait(Duration::from_millis(50))?,
                Err(TryLockError::Error(_)) => return Err("Could not lock Claude accounts".into()),
            }
        }
    }

    fn record_path(&self, id: &str) -> Result<PathBuf, String> {
        let digest = native::sha256(id.as_bytes(), &Cancellation::default())?;
        Ok(self
            .directory
            .join(format!("account-{}.json", hex(&digest))))
    }

    pub fn save_record(&self, record: &Record) -> Result<(), String> {
        self.atomic(&self.record_path(&record.id)?, &record.value())
    }

    pub fn record(&self, id: &str) -> Result<Record, String> {
        let path = self.record_path(id)?;
        let value = read_json(&path)?.ok_or("Claude account is not saved")?;
        let record = Record::parse(&value)?;
        if record.id != id {
            return Err("Claude account identity mismatch".into());
        }
        Ok(record)
    }

    pub fn records(&self) -> Result<Vec<Record>, String> {
        Ok(self.scan()?.0)
    }

    pub fn secrets_report(&self) -> Result<(Vec<String>, usize), String> {
        let (_, secrets, invalid) = self.scan()?;
        Ok((secrets, invalid))
    }

    fn scan(&self) -> Result<(Vec<Record>, Vec<String>, usize), String> {
        if !self.directory.exists() {
            return Ok((vec![], vec![], 0));
        }
        self.ensure()?;
        let mut records = Vec::new();
        let mut secrets = Vec::new();
        let mut invalid = 0;
        for entry in fs::read_dir(&self.directory).map_err(|_| "Could not list Claude accounts")? {
            let entry = entry.map_err(|_| "Could not list Claude accounts")?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("account-") || !name.ends_with(".json") {
                continue;
            }
            let value = match read_json(&entry.path()) {
                Ok(Some(value)) => value,
                Ok(None) => {
                    invalid += 1;
                    continue;
                }
                Err(error)
                    if matches!(
                        error.as_str(),
                        "Invalid Claude account JSON"
                            | "Invalid Claude account encoding"
                            | "Invalid Claude account record"
                            | "Claude account record exceeds size limit"
                    ) =>
                {
                    invalid += 1;
                    continue;
                }
                Err(error) => return Err(error),
            };
            for name in ["access_token", "refresh_token"] {
                if let Some(secret) = value.get(name).and_then(Value::as_str) {
                    secrets.push(secret.to_owned());
                }
            }
            if let Ok(record) = Record::parse(&value) {
                if self.record_path(&record.id)? == entry.path() {
                    records.push(record);
                } else {
                    invalid += 1;
                }
            } else {
                invalid += 1;
            }
        }
        Ok((records, secrets, invalid))
    }

    pub fn active(&self) -> Result<Option<String>, String> {
        let Some(value) = read_json(&self.directory.join("active.json"))? else {
            return Ok(None);
        };
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .ok_or("Invalid active Claude account")?;
        Ok((!id.is_empty()).then(|| id.to_owned()))
    }

    pub fn set_active(&self, id: Option<&str>) -> Result<(), String> {
        self.atomic(
            &self.directory.join("active.json"),
            &Value::object([("id", Value::string(id.unwrap_or("")))]),
        )
    }

    fn atomic(&self, path: &Path, value: &Value) -> Result<(), String> {
        self.ensure()?;
        if path.exists() {
            check_path(path, false)?;
        }
        let bytes = format!("{}\n", value.encode());
        if bytes.len() > LIMIT {
            return Err("Claude account record exceeds size limit".into());
        }
        let random = native::random(12, &Cancellation::default())?;
        let temporary = self.directory.join(format!(".pending-{}", hex(&random)));
        #[cfg(unix)]
        let file = {
            use std::os::unix::fs::OpenOptionsExt;
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)
        };
        #[cfg(windows)]
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary);
        let mut file = file.map_err(|_| "Could not create Claude account record")?;
        let result = (|| {
            protect_file(&temporary)?;
            file.write_all(bytes.as_bytes())
                .and_then(|_| file.sync_all())
                .map_err(|_| "Could not write Claude account record")?;
            drop(file);
            fs::rename(&temporary, path).map_err(|_| "Could not replace Claude account record")?;
            protect_file(path)?;
            #[cfg(unix)]
            File::open(&self.directory)
                .and_then(|dir| dir.sync_all())
                .map_err(|_| "Could not sync Claude account directory")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

pub(super) fn now() -> Result<u64, String> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "System clock is invalid")?
        .as_secs())
}

fn read_json(path: &Path) -> Result<Option<Value>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Could not inspect Claude account record".into()),
    };
    if !metadata.is_file() || metadata.len() > LIMIT as u64 {
        return Err("Invalid Claude account record".into());
    }
    check_path(path, false)?;
    protect_file(path)?;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| "Could not read Claude account record")?
        .take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read Claude account record")?;
    if bytes.len() > LIMIT {
        return Err("Claude account record exceeds size limit".into());
    }
    let text = String::from_utf8(bytes).map_err(|_| "Invalid Claude account encoding")?;
    Ok(Some(
        json::parse(&text).map_err(|_| "Invalid Claude account JSON")?,
    ))
}

fn check_path(path: &Path, directory: bool) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "Could not inspect Claude account path")?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Claude account path contains a link or junction".into());
        }
    }
    if metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err("Claude account path has an unexpected type".into());
    }
    Ok(())
}

fn protect_file(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| "Could not protect Claude account record")?;
    }
    #[cfg(windows)]
    native::protect(path, false)?;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
