use super::{
    Client, Error, auth,
    credentials::{self, Credentials},
    login, unix_seconds,
};
use crate::{
    state::Store,
    tls::{Budget, trust::TrustStore},
};
use std::ops::ControlFlow;

impl Client {
    /// Reuse or refresh the saved account. Device sign-in is needed only without usable credentials.
    pub fn connect(
        budget: &Budget<'_>,
        code: impl FnMut(&str) -> ControlFlow<()>,
    ) -> Result<Self, Error> {
        budget.check()?;
        let store = Store::user().map_err(|_| Error::Storage)?;
        // A sign-in queued behind another instance's logout must not recreate
        // credentials after that logout completed.
        let before = store
            .read("credentials.json", 32768)
            .map_err(|_| Error::Storage)?;
        let credentials = acquire_for_connect(store.clone(), budget, before.as_deref())?;
        let trust = TrustStore::native().map_err(|_| Error::Trust)?;
        let tokens = match credentials.load()? {
            Some(tokens) if fresh(&tokens)? => tokens,
            Some(tokens) => match refresh(&tokens, &credentials, &trust, budget) {
                Ok(tokens) => tokens,
                Err(Error::Login(auth::Error::Denied)) => {
                    let tokens = login::run(&trust, budget, code)?;
                    credentials.save(&tokens)?;
                    tokens
                }
                Err(error) => return Err(error),
            },
            None => {
                let tokens = login::run(&trust, budget, code)?;
                credentials.save(&tokens)?;
                tokens
            }
        };
        Ok(Self {
            trust,
            tokens,
            store: Some(store),
        })
    }

    pub fn logout(budget: &Budget<'_>) -> Result<(), Error> {
        Self::logout_in(Store::user().map_err(|_| Error::Storage)?, budget)
    }

    pub(super) fn logout_in(store: Store, budget: &Budget<'_>) -> Result<(), Error> {
        Credentials::open(store, budget)?.logout()
    }

    pub(super) fn ensure_access(&mut self, budget: &Budget<'_>) -> Result<(), Error> {
        let Some(store) = &self.store else {
            return Ok(());
        };
        let credentials = Credentials::open(store.clone(), budget)?;
        let tokens = credentials.load()?.ok_or(Error::AccountChanged)?;
        if !same_sign_in(&self.tokens, &tokens) {
            return Err(Error::AccountChanged);
        }
        self.tokens = if fresh(&tokens)? {
            tokens
        } else {
            refresh(&tokens, &credentials, &self.trust, budget)?
        };
        Ok(())
    }
}

pub(super) fn acquire_for_connect(
    store: Store,
    budget: &Budget<'_>,
    before: Option<&str>,
) -> Result<Credentials, Error> {
    let credentials = Credentials::open(store.clone(), budget)?;
    let after = store
        .read("credentials.json", 32768)
        .map_err(|_| Error::Storage)?;
    if credentials::changed_to_signed_out(before, after.as_deref()) {
        return Err(Error::AccountChanged);
    }
    Ok(credentials)
}

fn same_sign_in(first: &auth::Tokens, second: &auth::Tokens) -> bool {
    first.account_id() == second.account_id() && first.generation() == second.generation()
}

fn fresh(tokens: &auth::Tokens) -> Result<bool, Error> {
    Ok(unix_seconds()? < tokens.expires_at().saturating_sub(60))
}

fn refresh(
    tokens: &auth::Tokens,
    credentials: &Credentials,
    trust: &TrustStore,
    budget: &Budget<'_>,
) -> Result<auth::Tokens, Error> {
    let request = tokens.refresh_request()?;
    budget.check()?;
    credentials.refreshing()?;
    let reply = login::exchange(trust, &request, budget)?;
    let next = tokens.refreshed(reply.status, &reply.body, unix_seconds()?)?;
    credentials.save(&next)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_new_login_to_the_same_provider_account_invalidates_an_old_client() {
        let token = "e30.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjb3VudC10ZXN0In19.c2ln";
        let saved = |generation: &str, access: &str| {
            format!(
                r#"{{"version":1,"state":"ready","provider":"openai-account","access_token":"{access}","refresh_token":"synthetic","account_id":"account-test","expires_at":9999999999,"generation":"{generation}"}}"#
            )
        };
        let old = auth::Tokens::from_saved_json(&saved("a1", token))
            .unwrap()
            .unwrap();
        let refreshed = auth::Tokens::from_saved_json(&saved("a1", token))
            .unwrap()
            .unwrap();
        let new_login = auth::Tokens::from_saved_json(&saved("b2", token))
            .unwrap()
            .unwrap();
        assert!(same_sign_in(&old, &refreshed));
        assert!(!same_sign_in(&old, &new_login));
    }
}
