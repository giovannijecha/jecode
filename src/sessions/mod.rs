mod context;
mod delete;
mod document;
mod handle;
mod journal;
mod records;
mod storage;
mod summary;

pub(crate) use delete::DeleteReport;
pub use document::Draft;
pub use document::{Document, Input, Pending};
pub use handle::{Handle, Stage};
pub use records::Record;
pub use storage::{Store, Summary};

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

pub(crate) fn identifier() -> String {
    format!(
        "{}-{}-{}",
        now(),
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

fn canonical(path: &Path) -> Result<PathBuf, String> {
    let path = std::fs::canonicalize(path)
        .map_err(|error| format!("Could not resolve session directory: {error}"))?;
    if !path.is_dir() {
        return Err("Session directory must be a directory".into());
    }
    Ok(path)
}

fn directory_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn same_directory(a: &Path, b: &Path) -> bool {
    canonical(a)
        .ok()
        .zip(canonical(b).ok())
        .is_some_and(|(a, b)| directory_key(&a) == directory_key(&b))
}

#[cfg(test)]
mod crash_tests;
#[cfg(test)]
mod journal_tests;
#[cfg(all(test, windows))]
mod measurement_tests;
#[cfg(test)]
mod tests;
