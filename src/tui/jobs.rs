use crate::auth::openai::{SignInUpdate, Store};
use crate::{
    backend::{Endpoints, Model, Services},
    cancel::Cancellation,
    effort::Effort,
};
use std::thread::{self, JoinHandle};
use std::{path::PathBuf, sync::mpsc::Sender};

pub enum CatalogPurpose {
    Models {
        defaults: bool,
        id: Option<String>,
    },
    Effort {
        defaults: bool,
        model: String,
        effort: Option<Effort>,
    },
}
/// The routing selector an endpoint listing was requested for.
pub struct RoutingPurpose {
    pub defaults: bool,
    pub model: String,
}
pub enum ResultValue {
    Catalog(CatalogPurpose, Result<Vec<Model>, String>),
    Key(String, Result<(), String>),
    Endpoints(RoutingPurpose, Result<Option<Endpoints>, String>),
    SignIn(Result<crate::auth::openai::Account, String>),
    SignOut(Result<Option<String>, String>),
}
pub struct Job {
    task: Option<JoinHandle<ResultValue>>,
    cancellation: Cancellation,
    detach_on_cancel: bool,
}

impl Job {
    pub fn catalog(services: Services, purpose: CatalogPurpose) -> Self {
        Self::start(move |cancellation| {
            ResultValue::Catalog(purpose, services.with_cancellation(cancellation).catalog())
        })
    }
    pub fn endpoints(services: Services, purpose: RoutingPurpose) -> Self {
        Self::start(move |cancellation| {
            let result = services
                .with_cancellation(cancellation)
                .endpoints(&purpose.model);
            ResultValue::Endpoints(purpose, result)
        })
    }
    pub fn key(services: Services, key: String) -> Self {
        Self::start(move |cancellation| {
            ResultValue::Key(
                key,
                services.with_cancellation(cancellation).check_credential(),
            )
        })
    }
    pub fn sign_in(home: PathBuf, existing: Option<String>, updates: Sender<SignInUpdate>) -> Self {
        let mut job = Self::start(move |cancellation| {
            let store = Store::new(home);
            let result = store
                .sign_in(existing.as_deref(), cancellation, &mut |update| {
                    updates
                        .send(update)
                        .map_err(|_| "Sign-in cancelled".to_string())
                })
                .and_then(|session| session.account());
            ResultValue::SignIn(result)
        });
        job.detach_on_cancel = true;
        job
    }
    pub fn sign_out(home: PathBuf, subject: String) -> Self {
        Self::start(move |cancellation| {
            let store = Store::new(home);
            let result = store
                .session(&subject)
                .and_then(|session| session.sign_out(&cancellation));
            ResultValue::SignOut(result)
        })
    }
    fn start(work: impl FnOnce(Cancellation) -> ResultValue + Send + 'static) -> Self {
        let cancellation = Cancellation::default();
        let token = cancellation.clone();
        Self {
            task: Some(thread::spawn(move || work(token))),
            cancellation,
            detach_on_cancel: false,
        }
    }
    pub fn finished(&self) -> bool {
        self.task.as_ref().unwrap().is_finished()
    }
    pub fn finish(mut self) -> Result<ResultValue, String> {
        self.task
            .take()
            .unwrap()
            .join()
            .map_err(|_| "Settings worker failed".into())
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            self.cancellation.cancel();
            if !self.detach_on_cancel {
                let _ = task.join();
            }
        }
    }
}
