//! Directory-scoped, bounded metadata export from a closed session.
use super::{Store, load, load_legacy, v2};
use crate::{providers::openai_account::client::Attempt, session::scope::Directory};
use std::io;

const MAX_EXPORTED_ATTEMPTS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttemptSource {
    Generation,
    Compaction,
}
impl AttemptSource {
    pub fn name(self) -> &'static str {
        match self {
            Self::Generation => "generation",
            Self::Compaction => "compaction",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticAttempt {
    /// One-based canonical turn count. Request numbers restart per command.
    pub turn: usize,
    pub source: AttemptSource,
    pub attempt: Attempt,
}

/// The caller must close the session first so its normal lease can be held.
pub fn recent_network_attempts_in(
    id: &str,
    directory: &Directory,
) -> io::Result<Vec<DiagnosticAttempt>> {
    recent_network_attempts_in_store(&Store::user()?, id, directory)
}
pub(super) fn recent_network_attempts_in_store(
    store: &Store,
    id: &str,
    directory: &Directory,
) -> io::Result<Vec<DiagnosticAttempt>> {
    // Check scope before taking a foreign session's lease, then recheck the
    // authoritative committed snapshot while holding the correct version lease.
    let overview = load(store, id, false)?;
    directory.require(overview.directory.as_deref())?;
    drop(overview);
    let saved = if v2::has_head(store, id)? {
        v2::inspect_leased(store, id)?
    } else {
        load_legacy(store, id, true, false)?
    };
    directory.require(saved.directory.as_deref())?;

    let turn = saved.history.turn_count();
    let mut attempts: Vec<_> = saved
        .history
        .turns
        .last()
        .into_iter()
        .flat_map(|turn| turn.steps.iter().rev())
        .flat_map(|step| step.attempts.iter().rev())
        .take(MAX_EXPORTED_ATTEMPTS)
        .cloned()
        .map(|attempt| DiagnosticAttempt {
            turn,
            source: AttemptSource::Generation,
            attempt,
        })
        .collect();
    attempts.reverse();

    // Manual compaction follows the completed turn; automatic compaction
    // follows its preceding generation steps. An old unscoped failure can be
    // assigned only when the session has a single turn; no later turn exists.
    if saved.history.projection.failed
        && (saved.history.projection.failed_at_turn == Some(turn)
            || saved.history.projection.failed_at_turn.is_none() && turn == 1)
    {
        attempts.extend(
            saved
                .history
                .projection
                .failed_attempts
                .iter()
                .rev()
                .take(MAX_EXPORTED_ATTEMPTS)
                .rev()
                .cloned()
                .map(|attempt| DiagnosticAttempt {
                    turn,
                    source: AttemptSource::Compaction,
                    attempt,
                }),
        );
    }
    let excess = attempts.len().saturating_sub(MAX_EXPORTED_ATTEMPTS);
    attempts.drain(..excess);
    Ok(attempts)
}
