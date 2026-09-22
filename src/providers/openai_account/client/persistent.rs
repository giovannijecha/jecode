use super::{Client, Error, auth, credentials::Credentials, login, unix_seconds};
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
        let credentials = Credentials::open(store.clone(), budget)?;
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
        Credentials::open(Store::user().map_err(|_| Error::Storage)?, budget)?.logout()
    }

    pub(super) fn ensure_access(&mut self, budget: &Budget<'_>) -> Result<(), Error> {
        let Some(store) = &self.store else {
            return Ok(());
        };
        let credentials = Credentials::open(store.clone(), budget)?;
        let tokens = credentials.load()?.ok_or(Error::AccountChanged)?;
        if tokens.account_id() != self.tokens.account_id() {
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
