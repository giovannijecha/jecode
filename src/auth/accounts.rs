//! Account management shared by native subscription providers.

use super::{Credential, anthropic, openai};
use crate::cancel::Cancellation;
use crate::redact::Redactor;
use std::path::PathBuf;

pub const BACKENDS: &[&str] = &["openai", "anthropic"];

pub fn account_backend(id: &str) -> bool {
    BACKENDS.contains(&id)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub identity: String,
    pub label: String,
    pub detail: String,
    pub enabled: bool,
}

impl From<openai::Account> for Account {
    fn from(account: openai::Account) -> Self {
        Self {
            identity: format!("{}:{}", account.client_id, account.subject),
            id: account.client_id,
            label: account.label,
            detail: account.email,
            enabled: account.enabled,
        }
    }
}

impl From<anthropic::Account> for Account {
    fn from(account: anthropic::Account) -> Self {
        Self {
            identity: account.id.clone(),
            id: account.id,
            label: account.label,
            detail: String::new(),
            enabled: account.enabled,
        }
    }
}

pub enum SignInUpdate {
    Browser(String),
    Waiting,
}

#[derive(Clone)]
pub enum AccountSession {
    OpenAi(openai::Session),
    Anthropic(anthropic::Session),
}

impl AccountSession {
    pub fn backend(&self) -> &'static str {
        match self {
            Self::OpenAi(_) => "openai",
            Self::Anthropic(_) => "anthropic",
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::OpenAi(session) => session.client_id(),
            Self::Anthropic(session) => session.id(),
        }
    }

    pub fn identity(&self) -> String {
        match self {
            Self::OpenAi(session) => session.identity(),
            Self::Anthropic(session) => session.identity(),
        }
    }

    pub fn account(&self) -> Result<Account, String> {
        match self {
            Self::OpenAi(session) => session.account().map(Account::from),
            Self::Anthropic(session) => session.account().map(Account::from),
        }
    }

    pub fn sign_out(&self, cancel: &Cancellation) -> Result<Option<String>, String> {
        match self {
            Self::OpenAi(session) => session.sign_out(cancel),
            Self::Anthropic(session) => session.sign_out(cancel),
        }
    }

    pub fn credential(&self) -> Credential {
        match self {
            Self::OpenAi(session) => Credential::openai(session.clone()),
            Self::Anthropic(session) => Credential::anthropic(session.clone()),
        }
    }
}

#[derive(Clone)]
pub struct AccountStore {
    home: PathBuf,
    backend: String,
}

impl AccountStore {
    pub fn new(home: PathBuf, backend: &str) -> Self {
        Self {
            home,
            backend: backend.into(),
        }
    }

    fn unknown(&self) -> String {
        format!("Unknown account provider: {}", self.backend)
    }

    pub fn accounts(&self) -> Result<Vec<Account>, String> {
        match self.backend.as_str() {
            "openai" => openai::Store::new(self.home.clone())
                .accounts()
                .map(|accounts| accounts.into_iter().map(Account::from).collect()),
            "anthropic" => anthropic::Store::new(self.home.clone())
                .accounts()
                .map(|accounts| accounts.into_iter().map(Account::from).collect()),
            _ => Err(self.unknown()),
        }
    }

    pub fn active(&self) -> Result<Option<AccountSession>, String> {
        match self.backend.as_str() {
            "openai" => openai::Store::new(self.home.clone())
                .active()
                .map(|session| session.map(AccountSession::OpenAi)),
            "anthropic" => anthropic::Store::new(self.home.clone())
                .active()
                .map(|session| session.map(AccountSession::Anthropic)),
            _ => Err(self.unknown()),
        }
    }

    pub fn session(&self, id: &str) -> Result<AccountSession, String> {
        match self.backend.as_str() {
            "openai" => openai::Store::new(self.home.clone())
                .session(id)
                .map(AccountSession::OpenAi),
            "anthropic" => anthropic::Store::new(self.home.clone())
                .session(id)
                .map(AccountSession::Anthropic),
            _ => Err(self.unknown()),
        }
    }

    pub fn activate(&self, id: &str) -> Result<(), String> {
        match self.backend.as_str() {
            "openai" => openai::Store::new(self.home.clone()).activate(id),
            "anthropic" => anthropic::Store::new(self.home.clone()).activate(id),
            _ => Err(self.unknown()),
        }
    }

    pub fn sign_in(
        &self,
        existing: Option<&str>,
        cancel: Cancellation,
        emit: &mut dyn FnMut(SignInUpdate) -> Result<(), String>,
    ) -> Result<AccountSession, String> {
        match self.backend.as_str() {
            "openai" => openai::Store::new(self.home.clone())
                .sign_in(existing, cancel, &mut |update| {
                    emit(match update {
                        openai::SignInUpdate::Browser(url) => SignInUpdate::Browser(url),
                        openai::SignInUpdate::Waiting => SignInUpdate::Waiting,
                    })
                })
                .map(AccountSession::OpenAi),
            "anthropic" => anthropic::Store::new(self.home.clone())
                .sign_in(existing, cancel, &mut |update| {
                    emit(match update {
                        anthropic::SignInUpdate::Browser(url) => SignInUpdate::Browser(url),
                        anthropic::SignInUpdate::Waiting => SignInUpdate::Waiting,
                    })
                })
                .map(AccountSession::Anthropic),
            _ => Err(self.unknown()),
        }
    }

    pub fn redactor(&self) -> Result<Redactor, String> {
        match self.backend.as_str() {
            "openai" => openai::Store::new(self.home.clone()).redactor(),
            "anthropic" => anthropic::Store::new(self.home.clone()).redactor(),
            _ => Err(self.unknown()),
        }
    }

    pub(crate) fn redactor_report(&self) -> Result<(Redactor, usize), String> {
        match self.backend.as_str() {
            "openai" => openai::Store::new(self.home.clone()).redactor_report(),
            "anthropic" => anthropic::Store::new(self.home.clone()).redactor_report(),
            _ => Err(self.unknown()),
        }
    }
}

#[cfg(test)]
mod tests;
