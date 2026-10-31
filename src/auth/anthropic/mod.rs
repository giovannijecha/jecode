//! Claude account OAuth, isolated from API-key credentials and other clients.

mod flow;
mod storage;

use crate::cancel::Cancellation;
use crate::redact::Redactor;
use std::path::PathBuf;
use storage::{Files, Record};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub label: String,
    pub enabled: bool,
}

impl From<&Record> for Account {
    fn from(record: &Record) -> Self {
        Self {
            id: record.id.clone(),
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

    fn files(&self) -> Files {
        Files::new(&self.home)
    }

    pub fn redactor(&self) -> Result<Redactor, String> {
        Ok(self.redactor_report()?.0)
    }

    pub(crate) fn redactor_report(&self) -> Result<(Redactor, usize), String> {
        let redactor = Redactor::empty();
        let (secrets, invalid) = self.files().secrets_report()?;
        for secret in secrets {
            redactor.add_secret(secret);
        }
        Ok((redactor, invalid))
    }

    pub fn accounts(&self) -> Result<Vec<Account>, String> {
        let mut accounts: Vec<_> = self.files().records()?.iter().map(Account::from).collect();
        accounts.sort_by(|a, b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
        Ok(accounts)
    }

    pub fn session(&self, id: &str) -> Result<Session, String> {
        let record = self.files().record(id)?;
        Ok(self.make_session(&record))
    }

    pub fn active(&self) -> Result<Option<Session>, String> {
        self.files()
            .active()?
            .map(|id| self.session(&id))
            .transpose()
    }

    pub fn activate(&self, id: &str) -> Result<(), String> {
        let files = self.files();
        let _lock = files.lock(&Cancellation::default())?;
        if !files.record(id)?.enabled() {
            return Err("Claude account is signed out; sign in again".into());
        }
        files.set_active(Some(id))
    }

    pub fn sign_in(
        &self,
        existing: Option<&str>,
        cancel: Cancellation,
        emit: &mut dyn FnMut(SignInUpdate) -> Result<(), String>,
    ) -> Result<Session, String> {
        if let Some(id) = existing {
            self.files().record(id)?;
        }
        let record = flow::sign_in(&cancel, emit)?;
        if existing.is_some_and(|id| id != record.id) {
            return Err("Returning Claude account identity mismatch".into());
        }
        let files = self.files();
        let _lock = files.lock(&cancel)?;
        files.save_record(&record)?;
        Ok(self.make_session(&record))
    }

    fn make_session(&self, record: &Record) -> Session {
        let redactor = Redactor::empty();
        add_secrets(&redactor, record);
        Session {
            store: self.clone(),
            id: record.id.clone(),
            redactor,
            #[cfg(test)]
            _fixture_dir: None,
        }
    }

    #[cfg(test)]
    pub fn fixture_account(home: PathBuf, token: &str) -> Result<Account, String> {
        let store = Self::new(home);
        let record = Record {
            id: "fixture-account".into(),
            label: "Fixture".into(),
            access_token: token.into(),
            refresh_token: "fixture-refresh".into(),
            expires_at: u64::MAX,
            scopes: "user:profile user:inference".into(),
        };
        store.files().save_record(&record)?;
        store.activate(&record.id)?;
        Ok(Account::from(&record))
    }
}

#[derive(Clone)]
pub struct Session {
    store: Store,
    id: String,
    redactor: Redactor,
    #[cfg(test)]
    _fixture_dir: Option<std::sync::Arc<crate::test_support::Directory>>,
}

impl Session {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn identity(&self) -> String {
        self.id.clone()
    }

    pub fn redactor(&self) -> Redactor {
        self.redactor.clone()
    }

    pub fn account(&self) -> Result<Account, String> {
        let record = self.store.files().record(&self.id)?;
        add_secrets(&self.redactor, &record);
        Ok(Account::from(&record))
    }

    pub fn access_token(&self, cancel: &Cancellation) -> Result<String, String> {
        if cancel.requested() {
            return Err("Operation cancelled".into());
        }
        let files = self.store.files();
        let _lock = files.lock(cancel)?;
        let mut record = files.record(&self.id)?;
        add_secrets(&self.redactor, &record);
        if !record.enabled() {
            return Err("Claude account is signed out; sign in again".into());
        }
        if record.expires_at <= storage::now()?.saturating_add(60) {
            record = match flow::refresh(&record, cancel) {
                Ok(record) => record,
                Err(error) if flow::invalid_grant(&error) => {
                    record.clear();
                    files.save_record(&record)?;
                    return Err("Claude session expired; sign in again".into());
                }
                Err(error) => return Err(error),
            };
            add_secrets(&self.redactor, &record);
            files.save_record(&record)?;
        }
        Ok(record.access_token)
    }

    pub fn sign_out(&self, _cancel: &Cancellation) -> Result<Option<String>, String> {
        let files = self.store.files();
        let _lock = files.lock(&Cancellation::default())?;
        let mut record = files.record(&self.id)?;
        add_secrets(&self.redactor, &record);
        record.clear();
        files.save_record(&record)?;
        if files.active()?.as_deref() == Some(&self.id) {
            files.set_active(None)?;
        }
        Ok(None)
    }

    #[cfg(test)]
    pub fn fixture(token: &str) -> Self {
        let directory = std::sync::Arc::new(crate::test_support::Directory::new());
        let store = Store::new(directory.path().to_path_buf());
        let record = Record {
            id: "fixture-account".into(),
            label: "Fixture".into(),
            access_token: token.into(),
            refresh_token: "fixture-refresh".into(),
            expires_at: u64::MAX,
            scopes: "user:profile user:inference".into(),
        };
        store.files().save_record(&record).unwrap();
        let mut session = store.make_session(&record);
        session._fixture_dir = Some(directory);
        session
    }
}

fn add_secrets(redactor: &Redactor, record: &Record) {
    redactor.add_secret(record.access_token.clone());
    redactor.add_secret(record.refresh_token.clone());
}

#[cfg(test)]
mod tests;
