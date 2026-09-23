//! The lease orders credential checks and mutations, not model responses.
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
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::Interrupted => {
                    Error::Network(crate::tls::NetworkError::Cancelled)
                }
                _ => Error::Storage,
            })?;
        budget.check()?;
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
            .replace(
                "credentials.json",
                &format!(
                    r#"{{"version":1,"state":"signed_out","generation":"{}"}}"#,
                    auth::new_generation()
                ),
            )
            .map_err(|_| Error::Storage)
    }
}

pub(super) fn changed_to_signed_out(before: Option<&str>, after: Option<&str>) -> bool {
    before != after
        && after.is_some_and(|body| {
            crate::json::parse(body, Default::default())
                .ok()
                .and_then(|value| {
                    value
                        .get("state")
                        .and_then(crate::json::Value::text)
                        .map(str::to_owned)
                })
                .as_deref()
                == Some("signed_out")
        })
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

    #[test]
    fn logout_waits_for_in_flight_refresh_and_is_the_final_saved_state() {
        use std::sync::{Arc, Barrier};
        let fixture = crate::state::tests::Fixture::new();
        let Some(store) = fixture.store() else {
            return;
        };
        let token = "e30.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjb3VudC10ZXN0In19.c2ln";
        let json = format!(
            r#"{{"version":1,"state":"ready","provider":"openai-account","access_token":"{token}","refresh_token":"synthetic","account_id":"account-test","expires_at":1000,"generation":"a1"}}"#
        );
        let before = json.clone();
        let tokens = auth::Tokens::from_saved_json(&json).unwrap().unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let worker_store = store.clone();
        let worker_barrier = Arc::clone(&barrier);
        let worker = std::thread::spawn(move || {
            let cancelled = AtomicBool::new(false);
            let budget = Budget {
                cancelled: &cancelled,
                deadline: Instant::now() + Duration::from_secs(2),
            };
            let credentials = Credentials::open(worker_store, &budget).unwrap();
            credentials.refreshing().unwrap();
            worker_barrier.wait();
            std::thread::sleep(Duration::from_millis(30));
            credentials.save(&tokens).unwrap();
        });
        barrier.wait();
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(2),
        };
        Credentials::open(store.clone(), &budget)
            .unwrap()
            .logout()
            .unwrap();
        worker.join().unwrap();
        assert!(
            Credentials::open(store.clone(), &budget)
                .unwrap()
                .load()
                .unwrap()
                .is_none()
        );
        let signed_out = store.read("credentials.json", 32768).unwrap().unwrap();
        assert!(signed_out.contains("signed_out"));
        assert!(changed_to_signed_out(Some(&before), Some(&signed_out)));
        Credentials::open(store.clone(), &budget)
            .unwrap()
            .logout()
            .unwrap();
        let repeated = store.read("credentials.json", 32768).unwrap().unwrap();
        assert!(changed_to_signed_out(Some(&signed_out), Some(&repeated)));
    }
}
