use crate::{
    cancel::Cancellation,
    lsp::{self, CheckObservation, Profile},
};
use std::{
    path::PathBuf,
    thread::{self, JoinHandle},
};

pub(super) struct CheckJob {
    thread: Option<JoinHandle<Result<(String, CheckObservation), String>>>,
    cancel: Cancellation,
}

impl CheckJob {
    pub(super) fn start(profile: Profile, workspace: PathBuf) -> Self {
        let cancel = Cancellation::default();
        let token = cancel.clone();
        let thread = thread::spawn(move || {
            lsp::check(&profile, &workspace, &token).map(|observation| (profile.id, observation))
        });
        Self {
            thread: Some(thread),
            cancel,
        }
    }

    pub(super) fn finished(&self) -> bool {
        self.thread.as_ref().is_some_and(JoinHandle::is_finished)
    }

    pub(super) fn finish(mut self) -> Result<(String, CheckObservation), String> {
        self.thread
            .take()
            .unwrap()
            .join()
            .map_err(|_| "Language server check failed".to_string())?
    }
}

impl Drop for CheckJob {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
