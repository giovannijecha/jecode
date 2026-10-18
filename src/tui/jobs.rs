use crate::{
    cancel::Cancellation,
    effort::Effort,
    openrouter::{Api, Endpoints, Model},
};
use std::thread::{self, JoinHandle};

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
    Endpoints(RoutingPurpose, Result<Endpoints, String>),
}
pub struct Job {
    task: Option<JoinHandle<ResultValue>>,
    cancellation: Cancellation,
}

impl Job {
    pub fn catalog(api: Api, purpose: CatalogPurpose) -> Self {
        Self::start(move |cancellation| {
            ResultValue::Catalog(purpose, api.with_cancellation(cancellation).catalog())
        })
    }
    pub fn endpoints(api: Api, purpose: RoutingPurpose) -> Self {
        Self::start(move |cancellation| {
            let result = api
                .with_cancellation(cancellation)
                .endpoints(&purpose.model);
            ResultValue::Endpoints(purpose, result)
        })
    }
    pub fn key(api: Api, key: String) -> Self {
        Self::start(move |cancellation| {
            ResultValue::Key(key, api.with_cancellation(cancellation).check_key())
        })
    }
    fn start(work: impl FnOnce(Cancellation) -> ResultValue + Send + 'static) -> Self {
        let cancellation = Cancellation::default();
        let token = cancellation.clone();
        Self {
            task: Some(thread::spawn(move || work(token))),
            cancellation,
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
            let _ = task.join();
        }
    }
}
