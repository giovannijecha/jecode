use super::native;
use crate::cancel::Cancellation;
use crate::json::{self, Value};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const LIMIT: usize = 128 * 1024;

#[derive(Clone)]
pub(super) struct Record {
    pub client_id: String,
    pub subject: String,
    pub email: String,
    pub label: String,
    pub host_id: String,
    pub id_token: String,
    pub access_token: String,
    pub refresh_token: String,
    pub scopes: Vec<String>,
    pub expires_at: u64,
    pub earliest_refresh_at: u64,
}

impl Record {
    pub fn enabled(&self) -> bool {
        !self.access_token.is_empty()
            && !self.refresh_token.is_empty()
            && self
                .scopes
                .iter()
                .any(|scope| scope == "chatgpt.tokens.use.direct")
    }
    fn value(&self) -> Value {
        Value::object([
            ("format", Value::string("jecode-openai-account-v1")),
            ("client_id", Value::string(&self.client_id)),
            ("subject", Value::string(&self.subject)),
            ("email", Value::string(&self.email)),
            ("label", Value::string(&self.label)),
            ("ext_agent_host_id", Value::string(&self.host_id)),
            ("id_token", Value::string(&self.id_token)),
            ("access_token", Value::string(&self.access_token)),
            ("refresh_token", Value::string(&self.refresh_token)),
            (
                "scopes",
                Value::Array(self.scopes.iter().map(Value::string).collect()),
            ),
            ("expires_at", Value::number(self.expires_at)),
            (
                "earliest_refresh_at",
                Value::number(self.earliest_refresh_at),
            ),
        ])
    }
    fn parse(value: &Value) -> Result<Self, String> {
        let string = |key: &str| -> Result<String, String> {
            value
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| format!("Invalid account field {key}"))
        };
        let number = |key: &str| -> Result<u64, String> {
            match value.get(key) {
                Some(Value::Number(raw)) => raw
                    .parse()
                    .map_err(|_| format!("Invalid account field {key}")),
                _ => Err(format!("Invalid account field {key}")),
            }
        };
        if string("format")? != "jecode-openai-account-v1" {
            return Err("Unknown OpenAI account record".into());
        }
        let scopes = value
            .get("scopes")
            .and_then(Value::as_array)
            .ok_or("Invalid account scopes")?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or("Invalid account scope".into())
            })
            .collect::<Result<Vec<_>, String>>()?;
        let record = Self {
            client_id: string("client_id")?,
            subject: string("subject")?,
            email: string("email")?,
            label: string("label")?,
            host_id: string("ext_agent_host_id")?,
            id_token: string("id_token")?,
            access_token: string("access_token")?,
            refresh_token: string("refresh_token")?,
            scopes,
            expires_at: number("expires_at")?,
            earliest_refresh_at: number("earliest_refresh_at")?,
        };
        if record.client_id.is_empty()
            || record.subject.is_empty()
            || !record.host_id.starts_with("urn:uuid:")
        {
            return Err("Invalid OpenAI account identity".into());
        }
        Ok(record)
    }
}

pub(super) struct Files {
    pub directory: PathBuf,
}

impl Files {
    pub fn new(home: PathBuf) -> Self {
        Self {
            directory: home.join("accounts").join("openai"),
        }
    }

    pub fn ensure(&self) -> Result<(), String> {
        let parent = self
            .directory
            .parent()
            .ok_or("Invalid OpenAI account path")?;
        if parent.exists() {
            check_path(parent, true)?;
        }
        fs::create_dir_all(&self.directory)
            .map_err(|_| "Could not create OpenAI account directory")?;
        check_path(&self.directory, true)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.directory, fs::Permissions::from_mode(0o700))
                .map_err(|_| "Could not protect OpenAI account directory")?;
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
            .map_err(|_| "Could not open OpenAI account lock")?;
        protect_file(&path)?;
        loop {
            if cancel.requested() {
                return Err("Operation cancelled".into());
            }
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(TryLockError::WouldBlock) => cancel.wait(Duration::from_millis(50))?,
                Err(TryLockError::Error(_)) => return Err("Could not lock OpenAI accounts".into()),
            }
        }
    }

    pub fn host_id(&self, cancel: &Cancellation) -> Result<String, String> {
        let _lock = self.lock(cancel)?;
        let path = self.directory.join("host.json");
        if let Some(value) = read_json(&path)? {
            let id = value
                .get("ext_agent_host_id")
                .and_then(Value::as_str)
                .ok_or("Invalid OpenAI host identity")?;
            if !valid_uuid(id) {
                return Err("Invalid OpenAI host identity".into());
            }
            return Ok(id.into());
        }
        let mut bytes = native::random(16, cancel)?;
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let hex = hex(&bytes);
        let id = format!(
            "urn:uuid:{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        );
        self.atomic(
            &path,
            &Value::object([("ext_agent_host_id", Value::string(&id))]),
        )?;
        Ok(id)
    }

    fn record_path(
        &self,
        client_id: &str,
        subject: &str,
        cancel: &Cancellation,
    ) -> Result<PathBuf, String> {
        let mut identity = Vec::new();
        identity.extend_from_slice(client_id.as_bytes());
        identity.push(0);
        identity.extend_from_slice(subject.as_bytes());
        Ok(self.directory.join(format!(
            "account-{}.json",
            hex(&native::sha256(&identity, cancel)?)
        )))
    }

    pub fn save_record(&self, record: &Record, cancel: &Cancellation) -> Result<(), String> {
        self.ensure()?;
        let path = self.record_path(&record.client_id, &record.subject, cancel)?;
        self.atomic(&path, &record.value())
    }

    pub fn record(
        &self,
        client_id: &str,
        subject: &str,
        cancel: &Cancellation,
    ) -> Result<Record, String> {
        let path = self.record_path(client_id, subject, cancel)?;
        let record = Record::parse(&read_json(&path)?.ok_or("OpenAI account is not saved")?)?;
        if record.client_id != client_id || record.subject != subject {
            return Err("OpenAI account identity mismatch".into());
        }
        Ok(record)
    }

    pub fn records(&self) -> Result<Vec<Record>, String> {
        if !self.directory.exists() {
            return Ok(vec![]);
        }
        self.ensure()?;
        let mut records = vec![];
        for entry in fs::read_dir(&self.directory).map_err(|_| "Could not list OpenAI accounts")? {
            let entry = entry.map_err(|_| "Could not list OpenAI accounts")?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("account-") || !name.ends_with(".json") {
                continue;
            }
            let value = read_json(&entry.path())?.ok_or("Missing OpenAI account record")?;
            records.push(Record::parse(&value)?);
        }
        Ok(records)
    }

    pub fn active(&self) -> Result<Option<(String, String)>, String> {
        let path = self.directory.join("active.json");
        let Some(value) = read_json(&path)? else {
            return Ok(None);
        };
        let client = value
            .get("client_id")
            .and_then(Value::as_str)
            .ok_or("Invalid active OpenAI account")?;
        let subject = value
            .get("subject")
            .and_then(Value::as_str)
            .ok_or("Invalid active OpenAI account")?;
        Ok(Some((client.into(), subject.into())))
    }

    pub fn set_active(&self, selected: Option<(&str, &str)>) -> Result<(), String> {
        self.ensure()?;
        let value = match selected {
            Some((client, subject)) => Value::object([
                ("client_id", Value::string(client)),
                ("subject", Value::string(subject)),
            ]),
            None => Value::object([
                ("client_id", Value::string("")),
                ("subject", Value::string("")),
            ]),
        };
        self.atomic(&self.directory.join("active.json"), &value)
    }

    fn atomic(&self, path: &Path, value: &Value) -> Result<(), String> {
        self.ensure()?;
        if path.exists() {
            check_path(path, false)?;
        }
        let bytes = format!("{}\n", value.encode()).into_bytes();
        if bytes.len() > LIMIT {
            return Err("OpenAI account record exceeds size limit".into());
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
        let mut file = file.map_err(|_| "Could not create OpenAI account record")?;
        let outcome = (|| {
            protect_file(&temporary)?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| "Could not write OpenAI account record")?;
            drop(file);
            fs::rename(&temporary, path).map_err(|_| "Could not replace OpenAI account record")?;
            protect_file(path)?;
            #[cfg(unix)]
            File::open(&self.directory)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| "Could not sync OpenAI account directory")?;
            Ok(())
        })();
        if outcome.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        outcome
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
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Could not inspect OpenAI account record".into()),
    };
    if !metadata.is_file() || metadata.len() > LIMIT as u64 {
        return Err("Invalid OpenAI account record".into());
    }
    check_path(path, false)?;
    protect_file(path)?;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| "Could not read OpenAI account record")?
        .take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read OpenAI account record")?;
    if bytes.len() > LIMIT {
        return Err("OpenAI account record exceeds size limit".into());
    }
    let text = String::from_utf8(bytes).map_err(|_| "Invalid OpenAI account record encoding")?;
    Ok(Some(
        json::parse(&text).map_err(|_| "Invalid OpenAI account record JSON")?,
    ))
}

fn check_path(path: &Path, directory: bool) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "Could not inspect OpenAI account path")?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("OpenAI account path contains a link or junction".into());
        }
    }
    if metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err("OpenAI account path has an unexpected type".into());
    }
    Ok(())
}

fn protect_file(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| "Could not protect OpenAI account record")?;
    }
    #[cfg(windows)]
    native::protect(path, false)?;
    Ok(())
}

fn valid_uuid(id: &str) -> bool {
    let Some(raw) = id.strip_prefix("urn:uuid:") else {
        return false;
    };
    let bytes = raw.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                *b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
        && bytes[14] == b'4'
        && b"89abAB".contains(&bytes[19])
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
