//! OpenAI public-client OAuth registration, account isolation and token refresh.
//! Native OS cryptography and HTTPS adapters protect the OAuth boundary.

mod flow;
mod jwt;
pub(super) mod native;
mod storage;
mod wire;

pub(crate) use flow::open_browser;

use crate::cancel::Cancellation;
use crate::redact::Redactor;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use storage::{Files, Record};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub client_id: String,
    pub subject: String,
    pub email: String,
    pub label: String,
    /// Whether retained tokens and the plan-use grant can authorize inference.
    pub enabled: bool,
}

impl From<&Record> for Account {
    fn from(record: &Record) -> Self {
        Self {
            client_id: record.client_id.clone(),
            subject: record.subject.clone(),
            email: record.email.clone(),
            label: record.label.clone(),
            enabled: record.enabled(),
        }
    }
}

pub enum SignInUpdate {
    Browser(String),
    Waiting,
}

#[derive(Clone)]
pub struct Store {
    home: PathBuf,
}

impl Store {
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }

    pub fn redactor(&self) -> Result<Redactor, String> {
        Ok(self.redactor_report()?.0)
    }

    pub(crate) fn redactor_report(&self) -> Result<(Redactor, usize), String> {
        let redactor = Redactor::empty();
        let scan = self.files().records_report()?;
        for secret in scan.secrets {
            redactor.add_secret(secret);
        }
        Ok((redactor, scan.invalid))
    }

    #[cfg(test)]
    pub fn fixture_account(home: PathBuf, token: &str) -> Result<Account, String> {
        let store = Self::new(home);
        let files = store.files();
        let record = Record {
            client_id: "fixture-client".into(),
            subject: "fixture-subject".into(),
            email: "fixture@example.test".into(),
            label: "Fixture".into(),
            host_id: files.host_id(&Cancellation::default())?,
            id_token: "fixture-id".into(),
            access_token: token.into(),
            refresh_token: "fixture-refresh".into(),
            scopes: vec!["chatgpt.tokens.use.direct".into()],
            expires_at: u64::MAX,
            earliest_refresh_at: 0,
        };
        files.save_record(&record, &Cancellation::default())?;
        store.activate(&record.client_id)?;
        Ok(Account::from(&record))
    }
    fn files(&self) -> Files {
        Files::new(self.home.clone())
    }

    pub fn accounts(&self) -> Result<Vec<Account>, String> {
        let mut accounts: Vec<_> = self.files().records()?.iter().map(Account::from).collect();
        let labels: Vec<_> = accounts
            .iter()
            .map(|account| account.label.clone())
            .collect();
        for account in &mut accounts {
            if labels
                .iter()
                .filter(|label| **label == account.label)
                .count()
                > 1
            {
                account.label = format!(
                    "{} ({}/{})",
                    account.label, account.client_id, account.subject
                );
            }
        }
        accounts.sort_by(|a, b| a.label.cmp(&b.label).then(a.client_id.cmp(&b.client_id)));
        Ok(accounts)
    }

    pub fn session(&self, client_id: &str) -> Result<Session, String> {
        let records: Vec<_> = self
            .files()
            .records()?
            .into_iter()
            .filter(|r| r.client_id == client_id)
            .collect();
        if records.len() != 1 {
            return Err(if records.is_empty() {
                "OpenAI account is not saved"
            } else {
                "OpenAI client ID matches multiple accounts"
            }
            .into());
        }
        Ok(self.make_session(&records[0]))
    }

    pub fn active(&self) -> Result<Option<Session>, String> {
        let Some((client_id, subject)) = self.files().active()? else {
            return Ok(None);
        };
        if client_id.is_empty() {
            return Ok(None);
        }
        let record = self
            .files()
            .record(&client_id, &subject, &Cancellation::default())?;
        Ok(Some(self.make_session(&record)))
    }

    pub fn activate(&self, client_id: &str) -> Result<(), String> {
        let files = self.files();
        let _lock = files.lock(&Cancellation::default())?;
        let records: Vec<_> = files
            .records()?
            .into_iter()
            .filter(|r| r.client_id == client_id)
            .collect();
        if records.len() != 1 {
            return Err("OpenAI account selection is ambiguous or missing".into());
        }
        if !records[0].enabled() {
            return Err("OpenAI account needs a plan-use grant and current credentials".into());
        }
        files.set_active(Some((&records[0].client_id, &records[0].subject)))
    }

    pub fn sign_in(
        &self,
        existing: Option<&str>,
        cancel: Cancellation,
        emit: &mut dyn FnMut(SignInUpdate) -> Result<(), String>,
    ) -> Result<Session, String> {
        let files = self.files();
        let previous = existing
            .map(|client| -> Result<Record, String> {
                let records: Vec<_> = files
                    .records()?
                    .into_iter()
                    .filter(|r| r.client_id == client)
                    .collect();
                if records.len() != 1 {
                    return Err("OpenAI account selection is ambiguous or missing".into());
                }
                Ok(records.into_iter().next().unwrap())
            })
            .transpose()?;
        let host = files.host_id(&cancel)?;
        if previous
            .as_ref()
            .is_some_and(|record| record.host_id != host)
        {
            return Err("Saved OpenAI account belongs to a different host identity".into());
        }
        let record = flow::sign_in(&host, previous.as_ref(), &cancel, emit)?;
        let _lock = files.lock(&cancel)?;
        files.save_record(&record, &cancel)?;
        Ok(self.make_session(&record))
    }

    fn make_session(&self, record: &Record) -> Session {
        let redactor = Redactor::empty();
        add_secrets(&redactor, record);
        Session {
            store: self.clone(),
            client_id: record.client_id.clone(),
            subject: record.subject.clone(),
            redactor,
            tokens: Arc::default(),
            #[cfg(test)]
            _fixture_dir: None,
        }
    }
}

#[derive(Clone)]
pub struct Session {
    store: Store,
    client_id: String,
    subject: String,
    redactor: Redactor,
    tokens: Arc<Mutex<TokenCache>>,
    #[cfg(test)]
    _fixture_dir: Option<std::sync::Arc<crate::test_support::Directory>>,
}

/// The last usable access token and the record file state it was read from.
/// A changed stamp sends the next lookup through the locked disk path again.
#[derive(Default)]
struct TokenCache {
    path: Option<PathBuf>,
    token: Option<(String, u64, Stamp)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed: (i64, i64),
    #[cfg(unix)]
    mode: u32,
    #[cfg(windows)]
    created: u64,
    #[cfg(windows)]
    attributes: u32,
}

fn stamp(path: &std::path::Path) -> Option<Stamp> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return None;
    }
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt as _;
    #[cfg(windows)]
    use std::os::windows::fs::MetadataExt as _;
    #[cfg(windows)]
    if metadata.file_attributes() & 0x400 != 0 {
        return None;
    }
    Some(Stamp {
        len: metadata.len(),
        modified: metadata.modified().ok(),
        #[cfg(unix)]
        device: metadata.dev(),
        #[cfg(unix)]
        inode: metadata.ino(),
        #[cfg(unix)]
        changed: (metadata.ctime(), metadata.ctime_nsec()),
        #[cfg(unix)]
        mode: metadata.mode(),
        #[cfg(windows)]
        created: metadata.creation_time(),
        #[cfg(windows)]
        attributes: metadata.file_attributes(),
    })
}

impl Session {
    #[cfg(test)]
    pub fn fixture(token: &str) -> Self {
        let directory = std::sync::Arc::new(crate::test_support::Directory::new());
        let store = Store::new(directory.path().to_path_buf());
        let record = Record {
            client_id: "fixture-client".into(),
            subject: "fixture-subject".into(),
            email: "fixture@example.test".into(),
            label: "Fixture".into(),
            host_id: "urn:uuid:01234567-89ab-4def-8abc-0123456789ab".into(),
            id_token: "fixture-id".into(),
            access_token: token.into(),
            refresh_token: "fixture-refresh".into(),
            scopes: vec!["chatgpt.tokens.use.direct".into()],
            expires_at: u64::MAX,
            earliest_refresh_at: 0,
        };
        store
            .files()
            .save_record(&record, &Cancellation::default())
            .unwrap();
        let mut session = store.make_session(&record);
        session._fixture_dir = Some(directory);
        session
    }
    pub fn client_id(&self) -> &str {
        &self.client_id
    }
    pub fn identity(&self) -> String {
        format!("{}:{}", self.client_id, self.subject)
    }
    pub fn redactor(&self) -> Redactor {
        self.redactor.clone()
    }

    pub fn account(&self) -> Result<Account, String> {
        let record =
            self.store
                .files()
                .record(&self.client_id, &self.subject, &Cancellation::default())?;
        add_secrets(&self.redactor, &record);
        Ok(Account::from(&record))
    }

    /// A valid access token. While the record file is unchanged and the token
    /// is not near expiry, this answers from memory: the disk path takes the
    /// account lock and, on Windows, launches native permission and hashing
    /// helpers, which cost hundreds of milliseconds per request.
    pub fn access_token(&self, cancel: &Cancellation) -> Result<String, String> {
        if cancel.requested() {
            return Err("Operation cancelled".into());
        }
        if let Some(token) = self.cached_token() {
            return Ok(token);
        }
        let token = self.stored_token(cancel)?;
        Ok(token)
    }

    fn cached_token(&self) -> Option<String> {
        let cache = self.tokens.lock().ok()?;
        let (token, expires_at, recorded) = cache.token.as_ref()?;
        let now = storage::now().ok()?;
        (*expires_at > now.saturating_add(60) && stamp(cache.path.as_ref()?) == Some(*recorded))
            .then(|| token.clone())
    }

    fn remember_token(&self, token: &str, expires_at: u64, cancel: &Cancellation) {
        let Ok(mut cache) = self.tokens.lock() else {
            return;
        };
        if cache.path.is_none() {
            cache.path = self
                .store
                .files()
                .record_path(&self.client_id, &self.subject, cancel)
                .ok();
        }
        cache.token = cache
            .path
            .as_deref()
            .and_then(stamp)
            .map(|recorded| (token.to_owned(), expires_at, recorded));
    }

    fn stored_token(&self, cancel: &Cancellation) -> Result<String, String> {
        if let Ok(mut cache) = self.tokens.lock() {
            cache.token = None;
        }
        let files = self.store.files();
        let _lock = files.lock(cancel)?;
        let mut record = files.record(&self.client_id, &self.subject, cancel)?;
        add_secrets(&self.redactor, &record);
        if !record.enabled() {
            return Err("OpenAI account is signed out or lacks the ChatGPT plan-use grant".into());
        }
        let now = storage::now()?;
        if record.expires_at <= now.saturating_add(60) {
            if now < record.earliest_refresh_at {
                if record.expires_at <= now {
                    return Err("OpenAI token expired; wait briefly before retrying".into());
                }
            } else {
                record = match flow::refresh(&record, cancel) {
                    Ok(updated) => updated,
                    Err(error) if unusable_refresh(&error) => {
                        record.access_token.clear();
                        record.refresh_token.clear();
                        record.id_token.clear();
                        record.expires_at = 0;
                        record.earliest_refresh_at = 0;
                        files.save_record(&record, cancel)?;
                        return Err("OpenAI session expired; sign in again".into());
                    }
                    Err(error) if wire::machine_code(&error) == Some("invalid_client") => {
                        return Err("OpenAI client configuration was rejected; check the saved registration".into());
                    }
                    Err(error) => return Err(error),
                };
                add_secrets(&self.redactor, &record);
                files.save_record(&record, cancel)?;
                if !record.enabled() {
                    return Err("OpenAI plan-use grant was not renewed; sign in again".into());
                }
            }
        }
        self.remember_token(&record.access_token, record.expires_at, cancel);
        Ok(record.access_token)
    }

    pub fn sign_out(&self, cancel: &Cancellation) -> Result<Option<String>, String> {
        let files = self.store.files();
        // Local removal remains mandatory even if remote revocation is cancelled.
        let previous = {
            let _lock = files.lock(&Cancellation::default())?;
            let mut record =
                files.record(&self.client_id, &self.subject, &Cancellation::default())?;
            add_secrets(&self.redactor, &record);
            let previous = record.clone();
            record.access_token.clear();
            record.refresh_token.clear();
            record.id_token.clear();
            record.expires_at = 0;
            record.earliest_refresh_at = 0;
            files.save_record(&record, &Cancellation::default())?;
            if files.active()?.is_some_and(|(client, subject)| {
                client == self.client_id && subject == self.subject
            }) {
                files.set_active(None)?;
            }
            if let Ok(mut cache) = self.tokens.lock() {
                cache.token = None;
            }
            previous
        };
        let warning = flow::revoke(&previous, cancel).err().map(|_| {
            "OpenAI revocation could not be confirmed; credentials were removed locally."
                .to_string()
        });
        Ok(warning)
    }
}

fn add_secrets(redactor: &Redactor, record: &Record) {
    redactor.add_secret(record.access_token.clone());
    redactor.add_secret(record.refresh_token.clone());
    redactor.add_secret(record.id_token.clone());
}

fn unusable_refresh(error: &str) -> bool {
    matches!(
        wire::machine_code(error),
        Some(
            "invalid_grant"
                | "invalid_refresh_token"
                | "token_expired"
                | "refresh_token_expired"
                | "refresh_token_invalidated"
                | "refresh_token_reused"
        )
    )
}

#[cfg(test)]
mod tests;
