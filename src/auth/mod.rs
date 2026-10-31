//! User-scoped credential material and its backend identity.
//!
//! Saved API keys and renewable account sessions share this boundary without
//! changing the provider-independent inference contract.

use std::process::Command;

mod accounts;
pub mod anthropic;
pub mod openai;
pub use accounts::{Account, AccountSession, AccountStore, SignInUpdate, account_backend};
pub(crate) use openai::open_browser;

/// A stable credential scope for saved measurements and provider-owned history.
/// Only the OS-computed digest is retained beside conversation data.
pub(crate) fn key_scope(
    key: &str,
    cancellation: &crate::cancel::Cancellation,
) -> Result<String, String> {
    let digest = openai::native::sha256(key.as_bytes(), cancellation)?;
    Ok(format!(
        "key-sha256:{}",
        crate::attachments::base64::encode(&digest)
    ))
}

/// Prevent backend credentials from reaching child tools and native helpers.
pub fn isolate_environment(command: &mut Command) -> &mut Command {
    for backend in crate::backend::available() {
        for name in backend.credential_env {
            command.env_remove(name);
        }
    }
    command
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub backend: String,
    pub account: Option<String>,
}

#[derive(Clone)]
pub enum Secret {
    ApiKey(String),
    AccountSession(AccountSession),
}

#[derive(Clone)]
pub struct Credential {
    pub identity: Identity,
    secret: Secret,
}

impl Credential {
    pub fn openai(session: openai::Session) -> Self {
        Self::account(AccountSession::OpenAi(session))
    }

    pub fn anthropic(session: anthropic::Session) -> Self {
        Self::account(AccountSession::Anthropic(session))
    }

    pub fn account(session: AccountSession) -> Self {
        Self {
            identity: Identity {
                backend: session.backend().into(),
                account: Some(session.identity()),
            },
            secret: Secret::AccountSession(session),
        }
    }

    pub fn account_session(&self) -> Option<&AccountSession> {
        match &self.secret {
            Secret::AccountSession(session) => Some(session),
            Secret::ApiKey(_) => None,
        }
    }

    pub fn openai_session(&self) -> Option<&openai::Session> {
        match self.account_session()? {
            AccountSession::OpenAi(session) => Some(session),
            AccountSession::Anthropic(_) => None,
        }
    }

    pub fn anthropic_session(&self) -> Option<&anthropic::Session> {
        match self.account_session()? {
            AccountSession::Anthropic(session) => Some(session),
            AccountSession::OpenAi(_) => None,
        }
    }

    pub fn api_key(backend: impl Into<String>, key: String) -> Self {
        Self {
            identity: Identity {
                backend: backend.into(),
                account: None,
            },
            secret: Secret::ApiKey(key),
        }
    }

    pub fn api_key_material(&self) -> Option<&str> {
        match &self.secret {
            Secret::ApiKey(key) => Some(key),
            Secret::AccountSession(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_environment_excludes_every_registered_backend_credential() {
        let mut command = Command::new("fixture-only");
        command.env("JECODE_TEST_UNRELATED", "kept");
        command.env("OPENAI_API_KEY", "another-app-key");
        command.env("ACCESS_TOKEN", "another-app-token");
        isolate_environment(&mut command);
        let env: Vec<_> = command.get_envs().collect();
        for name in crate::backend::available()
            .iter()
            .flat_map(|backend| backend.credential_env)
        {
            assert!(
                env.iter()
                    .any(|(key, value)| key.to_string_lossy() == *name && value.is_none())
            );
        }
        assert!(env.iter().any(|(key, value)| {
            key.to_string_lossy() == "JECODE_TEST_UNRELATED"
                && value == &Some(std::ffi::OsStr::new("kept"))
        }));
        for (name, value) in [
            ("OPENAI_API_KEY", "another-app-key"),
            ("ACCESS_TOKEN", "another-app-token"),
        ] {
            assert!(env.iter().any(|(key, set)| {
                key.to_string_lossy() == name && set == &Some(std::ffi::OsStr::new(value))
            }));
        }
    }
}
