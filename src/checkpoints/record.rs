//! Journal records: path states, observations, operations and points.
use super::files::Mode;
use super::frame::{Decoder, Encoder, Kind, encode};
use super::hash::Digest;

/// Observable file system state of one workspace path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Absent,
    File {
        object: u64,
        length: u64,
        digest: Digest,
        mode: Mode,
    },
    Directory {
        mode: Mode,
    },
    Link {
        target: String,
    },
    Oversized {
        length: u64,
    },
    Unreadable {
        reason: String,
    },
}

impl State {
    fn encode(&self, out: &mut Encoder) {
        match self {
            Self::Absent => {
                out.u8(0);
            }
            Self::File {
                object,
                length,
                digest,
                mode,
            } => {
                out.u8(1)
                    .u64(*object)
                    .u64(*length)
                    .digest(*digest)
                    .u64(u64::from(mode.0));
            }
            Self::Directory { mode } => {
                out.u8(2).u64(u64::from(mode.0));
            }
            Self::Link { target } => {
                out.u8(3).text(target);
            }
            Self::Oversized { length } => {
                out.u8(4).u64(*length);
            }
            Self::Unreadable { reason } => {
                out.u8(5).text(reason);
            }
        }
    }

    fn decode(input: &mut Decoder<'_>) -> Result<Self, String> {
        let mode = |value: u64| Mode(value as u32);
        Ok(match input.u8()? {
            0 => Self::Absent,
            1 => Self::File {
                object: input.u64()?,
                length: input.u64()?,
                digest: input.digest()?,
                mode: mode(input.u64()?),
            },
            2 => Self::Directory {
                mode: mode(input.u64()?),
            },
            3 => Self::Link {
                target: input.text()?,
            },
            4 => Self::Oversized {
                length: input.u64()?,
            },
            5 => Self::Unreadable {
                reason: input.text()?,
            },
            _ => return Err("unknown path state".into()),
        })
    }

    pub fn exists(&self) -> bool {
        !matches!(self, Self::Absent)
    }

    /// Whether this state's content is recoverable from the archive.
    pub fn recoverable(&self) -> bool {
        matches!(
            self,
            Self::Absent | Self::File { .. } | Self::Directory { .. } | Self::Link { .. }
        )
    }

    /// Equality of recoverable content and metadata, ignoring archive object ids.
    pub fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::File {
                    length: a,
                    digest: x,
                    mode: m,
                    ..
                },
                Self::File {
                    length: b,
                    digest: y,
                    mode: n,
                    ..
                },
            ) => a == b && x == y && m == n,
            _ => self == other,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::File { .. } => "file",
            Self::Directory { .. } => "directory",
            Self::Link { .. } => "link",
            Self::Oversized { .. } => "oversized",
            Self::Unreadable { .. } => "unreadable",
        }
    }
}

/// File system fingerprint used only to decide what to reread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stat {
    pub length: u64,
    pub modified: u64,
    pub changed: u64,
    pub inode: u64,
    pub device: u64,
    pub mode: u32,
}

impl Stat {
    pub fn of(metadata: &std::fs::Metadata) -> Self {
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |time| time.as_nanos() as u64);
        #[cfg(unix)]
        let (changed, inode, device) = {
            use std::os::unix::fs::MetadataExt;
            (
                (metadata.ctime() as u64)
                    .wrapping_mul(1_000_000_000)
                    .wrapping_add(metadata.ctime_nsec() as u64),
                metadata.ino(),
                metadata.dev(),
            )
        };
        // Windows exposes no stable change time; a replaced file has a new
        // creation time, and notifications cover in-place writes.
        #[cfg(windows)]
        let (changed, inode, device) = {
            use std::os::windows::fs::MetadataExt;
            (metadata.creation_time(), 0, 0)
        };
        #[cfg(not(any(unix, windows)))]
        let (changed, inode, device) = (0, 0, 0);
        Self {
            length: metadata.len(),
            modified,
            changed,
            inode,
            device,
            mode: Mode::of(metadata).0,
        }
    }
    fn encode(&self, out: &mut Encoder) {
        out.u64(self.length)
            .u64(self.modified)
            .u64(self.changed)
            .u64(self.inode)
            .u64(self.device)
            .u64(u64::from(self.mode));
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, String> {
        Ok(Self {
            length: input.u64()?,
            modified: input.u64()?,
            changed: input.u64()?,
            inode: input.u64()?,
            device: input.u64()?,
            mode: input.u64()? as u32,
        })
    }
}

/// How a path's first recorded version was discovered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discovery {
    /// Already recorded before; this is a later version.
    Known = 0,
    /// Appeared while coverage was complete, so it was absent before.
    Created = 1,
    /// Found by a baseline or a scope change; earlier existence is unknown.
    Found = 2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub seq: u64,
    pub path: String,
    pub state: State,
    pub stat: Option<Stat>,
    pub time: u64,
    /// Session or user that caused the observation, when known.
    pub origin: String,
    /// Operation (prepare sequence) that produced it, or 0 for an observation.
    pub operation: u64,
    pub discovery: Discovery,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Applied = 1,
    NotApplied = 2,
    Unknown = 3,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepare {
    pub seq: u64,
    pub owner: String,
    pub origin: String,
    pub kind: String,
    pub path: String,
    pub before: State,
    pub after: State,
    pub time: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Point {
    pub seq: u64,
    pub kind: String,
    pub label: String,
    pub origin: String,
    pub time: u64,
    pub policy: u64,
    pub pinned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub seq: u64,
    pub origin: String,
    pub time: u64,
    pub text: String,
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    Version(Version),
    Stat {
        seq: u64,
        path: String,
        stat: Stat,
    },
    Prepare(Prepare),
    Complete {
        seq: u64,
        operation: u64,
        outcome: Outcome,
    },
    Point(Point),
    Pin {
        seq: u64,
        target: u64,
        pinned: bool,
        label: String,
    },
    Policy {
        seq: u64,
        id: u64,
        text: String,
    },
    Event(Event),
    /// Complete coverage was established (`true`) or became uncertain.
    Coverage {
        seq: u64,
        complete: bool,
        reason: String,
    },
    /// Compaction moved the archive to a newer generation.
    Moved {
        seq: u64,
        generation: u64,
    },
}

impl Record {
    pub fn seq(&self) -> u64 {
        match self {
            Self::Version(version) => version.seq,
            Self::Prepare(prepare) => prepare.seq,
            Self::Point(point) => point.seq,
            Self::Event(event) => event.seq,
            Self::Stat { seq, .. }
            | Self::Complete { seq, .. }
            | Self::Pin { seq, .. }
            | Self::Policy { seq, .. }
            | Self::Coverage { seq, .. }
            | Self::Moved { seq, .. } => *seq,
        }
    }
}

mod codec;
#[cfg(test)]
mod tests;
