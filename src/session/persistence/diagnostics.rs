//! Directory-scoped, bounded metadata export from a closed session.
use super::{Store, resume_in_store};
use crate::{providers::openai_account::client::Attempt, session::scope::Directory};
use std::io;

/// The caller must close the session first so its normal lease can be held.
pub fn recent_network_attempts_in(id: &str, directory: &Directory) -> io::Result<Vec<Attempt>> {
    recent_network_attempts_in_store(&Store::user()?, id, directory)
}
pub(super) fn recent_network_attempts_in_store(
    store: &Store,
    id: &str,
    directory: &Directory,
) -> io::Result<Vec<Attempt>> {
    let saved = resume_in_store(store, id, directory)?;
    let mut attempts: Vec<_> = saved
        .history
        .turns
        .last()
        .into_iter()
        .flat_map(|turn| turn.steps.iter().rev())
        .flat_map(|step| step.attempts.iter().rev())
        .take(32)
        .cloned()
        .collect();
    if attempts.is_empty() && saved.history.projection.failed {
        attempts = saved
            .history
            .projection
            .failed_attempts
            .iter()
            .rev()
            .take(32)
            .cloned()
            .collect();
    }
    attempts.reverse();
    Ok(attempts)
}
