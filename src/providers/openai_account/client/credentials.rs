//! The lock covers read/refresh/replace. It never covers model generation.
use super::{Error, auth};
use crate::{
    state::{Lease, Store},
    tls::Budget,
};

pub(super) struct Credentials {
    store: Store,
    _lock: Lease,
}
impl Credentials {
    pub fn open(store: Store, budget: &Budget<'_>) -> Result<Self, Error> {
        budget.check()?;
        let lock = store
            .lock("credentials.lock", budget.cancelled, budget.deadline)
            .map_err(|_| Error::Storage)?;
        Ok(Self { store, _lock: lock })
    }

    pub fn load(&self) -> Result<Option<auth::Tokens>, Error> {
        self.store
            .read("credentials.json", 32768)
            .map_err(|_| Error::Storage)?
            .map(|body| auth::Tokens::from_saved_json(&body).map_err(|_| Error::Storage))
            .transpose()
            .map(Option::flatten)
    }

    pub fn save(&self, tokens: &auth::Tokens) -> Result<(), Error> {
        self.store
            .replace("credentials.json", &tokens.saved_json()?)
            .map_err(|_| Error::Storage)
    }

    /// Write before sending a rotating refresh token: a crash cannot replay it.
    pub fn refreshing(&self) -> Result<(), Error> {
        self.store
            .replace(
                "credentials.json",
                r#"{"version":1,"state":"reauth_required"}"#,
            )
            .map_err(|_| Error::Storage)
    }

    pub fn logout(&self) -> Result<(), Error> {
        self.store
            .replace("credentials.json", r#"{"version":1,"state":"signed_out"}"#)
            .map_err(|_| Error::Storage)
    }
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests {
    use super::*;
    use std::{
        sync::atomic::AtomicBool,
        time::{Duration, Instant},
    };

    #[test]
    fn interrupted_refresh_requires_new_login_and_legacy_is_untouched() {
        let fixture = crate::state::tests::Fixture::new();
        let Some(store) = fixture.store() else {
            return;
        };
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(1),
        };
        let token = "e30.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjb3VudC10ZXN0In19.c2ln";
        let json = format!(
            r#"{{"version":1,"state":"ready","provider":"openai-account","access_token":"{token}","refresh_token":"synthetic","account_id":"account-test","expires_at":1000}}"#
        );
        let tokens = auth::Tokens::from_saved_json(&json).unwrap().unwrap();
        let credentials = Credentials::open(store.clone(), &budget).unwrap();
        credentials.save(&tokens).unwrap();
        drop(credentials);
        let credentials = Credentials::open(store.clone(), &budget).unwrap();
        assert_eq!(credentials.load().unwrap().unwrap().access_token(), token);
        credentials.refreshing().unwrap();
        drop(credentials);
        let credentials = Credentials::open(store.clone(), &budget).unwrap();
        assert!(credentials.load().unwrap().is_none());
        assert!(
            !store
                .read("credentials.json", 32768)
                .unwrap()
                .unwrap()
                .contains("synthetic")
        );
        credentials.save(&tokens).unwrap();
        credentials.logout().unwrap();
        assert!(credentials.load().unwrap().is_none());
    }
}
