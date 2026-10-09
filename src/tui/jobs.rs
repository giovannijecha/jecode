use crate::{
    cancel::Cancellation,
    effort::Effort,
    openrouter::{Api, Model},
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
pub enum ResultValue {
    Catalog(CatalogPurpose, Result<Vec<Model>, String>),
    Key(String, Result<(), String>),
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
