//! Names of the session storage tree, kept in one place:
//!
//! ```text
//! <home>/sessions/<project-hash>/
//!   attachments/
//!   <session-id>/
//!     session.jsonl  session.lock  summary.json
//!     outputs/  tmp/  collaboration/
//! ```
//!
//! A collaborator's own home, `collaboration/child-home`, repeats this tree.

use std::path::{Path, PathBuf};

pub(crate) const SESSIONS: &str = "sessions";
pub(crate) const ATTACHMENTS: &str = "attachments";
pub(crate) const JOURNAL: &str = "session.jsonl";
pub(crate) const LOCK: &str = "session.lock";
pub(crate) const SUMMARY: &str = "summary.json";
pub(crate) const OUTPUTS: &str = "outputs";
pub(crate) const TEMPORARY: &str = "tmp";
pub(crate) const COLLABORATION: &str = "collaboration";
pub(crate) const CHILD_HOME: &str = "child-home";

/// The folder holding the sessions of the canonical project `directory`.
pub(super) fn bucket(home: &Path, directory: &Path) -> PathBuf {
    // A stable owned bucket calculation avoids path-length dependent names.
    // The canonical directory in each document is the authoritative scope.
    let key = super::directory_key(directory)
        .bytes()
        .fold(0u128, |key, byte| {
            key.wrapping_mul(257).wrapping_add(u128::from(byte) + 1)
        });
    home.join(SESSIONS).join(format!("{key:032x}"))
}

/// Whether `name` is a project bucket's name.
pub(crate) fn bucket_name(name: &str) -> bool {
    name.len() == 32 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
}
