//! The words of a request and of a grant (B12.4, B12.5), shared by the TUI
//! and the line-based CLI.

use super::{Access, Kind};

/// `Read access outside this folder`.
pub fn access(access: &Access) -> &'static str {
    match access.kind {
        Kind::Reads => "Read access outside this folder",
        Kind::Edits => "Write access outside this folder",
        Kind::Runs => "Runs outside this folder",
    }
}

/// The fact beside Approve: `Only this read`.
pub fn once(access: &Access) -> &'static str {
    match access.kind {
        Kind::Reads => "Only this read",
        Kind::Edits => "Only this edit",
        Kind::Runs => "Only this run",
    }
}

/// What Approve and remember covers, exactly: `Reads of this file`. The
/// option adds `, this conversation`; the result after it, `· remembered
/// for this conversation`.
pub fn covers(access: &Access, tool: &str) -> &'static str {
    match (access.kind, tool) {
        (Kind::Reads, "list" | "search") => "Reads of this folder",
        (Kind::Reads, "read") if access.members.len() > 1 => {
            match access.members.iter().filter(|file| file.outside).count() {
                count if count == access.members.len() => "Reads of these files",
                1 => "Reads of the outside file above",
                _ => "Reads of the outside files above",
            }
        }
        (Kind::Reads, _) => "Reads of this file",
        // One request names every file of a multi-file edit; only the outside
        // ones need remembering.
        (Kind::Edits, "edit") if access.members.len() > 1 => {
            match access.members.iter().filter(|file| file.outside).count() {
                count if count == access.members.len() => "Edits to these files",
                1 => "Edits to the outside file above",
                _ => "Edits to the outside files above",
            }
        }
        (Kind::Edits, _) => "Edits to this file",
        (Kind::Runs, _) => "This command there",
    }
}

pub const DENY: &str = "Nothing runs; the agent is told";
