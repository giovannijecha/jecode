//! Observing workspace paths and recording changed versions.
use super::files::Mode;
use super::index::Index;
use super::record::{Discovery, Record, Stat, State, Version};
use super::store::{Store, Transaction};
use std::fs;
use std::path::Path;
use std::sync::Arc;

#[cfg(test)]
mod tests;

pub const BATCH_BYTES: u64 = 8 << 20;
const BATCH_PATHS: usize = 1024;

/// One path's observed state, with the bytes needed to archive a file.
pub struct Observed {
    pub path: String,
    pub kind: Observation,
    pub stat: Option<Stat>,
}

pub enum Observation {
    Absent,
    File(Arc<[u8]>, Mode),
    Directory(Mode),
    Link(String),
    Oversized(u64),
    Unreadable(String),
}

/// Reads a path without following links. Never mutates the workspace.
pub fn observe(root: &Path, relative: &str, max_file_bytes: u64) -> Observed {
    let path = super::identity::absolute(root, relative);
    let observed = |kind, stat| Observed {
        path: relative.to_string(),
        kind,
        stat,
    };
    if let Err(reason) = super::files::confined_parent(root, relative) {
        return observed(Observation::Unreadable(reason), None);
    }
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return observed(Observation::Absent, None);
        }
        Err(error) => return observed(Observation::Unreadable(error.to_string()), None),
    };
    let stat = Some(Stat::of(&metadata));
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return match fs::read_link(&path) {
            Ok(target) => match target.to_str() {
                Some(target) => {
                    let target = if cfg!(windows) {
                        target.replace('\\', "/")
                    } else {
                        target.to_owned()
                    };
                    observed(Observation::Link(target), stat)
                }
                None => observed(
                    Observation::Unreadable("link target is not Unicode".into()),
                    stat,
                ),
            },
            Err(error) => observed(Observation::Unreadable(error.to_string()), stat),
        };
    }
    if file_type.is_dir() {
        return observed(Observation::Directory(Mode::of(&metadata)), stat);
    }
    if !file_type.is_file() {
        return observed(Observation::Unreadable("special file".into()), stat);
    }
    if metadata.len() > max_file_bytes {
        return observed(Observation::Oversized(metadata.len()), stat);
    }
    match read_bounded(&path, max_file_bytes) {
        Ok(Some(bytes)) => observed(Observation::File(bytes.into(), Mode::of(&metadata)), stat),
        Ok(None) => observed(
            Observation::Oversized(metadata.len().max(max_file_bytes + 1)),
            stat,
        ),
        Err(error) => observed(Observation::Unreadable(error.to_string()), stat),
    }
}

/// Delivers observations in path order, in bounded batches. Size hints from a
/// prior walk cannot bound live file sizes: files may grow before this read.
/// Each worker has one queued result, so a large restore or dirty burst never
/// retains the entire workspace's contents before recording its first batch.
pub fn observe_batches<P: AsRef<str> + Sync>(
    root: &Path,
    paths: &[P],
    max: u64,
    mut consume: impl FnMut(Vec<Observed>) -> Result<(), String>,
) -> Result<(), String> {
    std::thread::scope(|scope| {
        let workers = if paths.len() < 16 {
            0
        } else {
            super::scan::WORKERS.min(paths.len())
        };
        let mut receivers = Vec::with_capacity(workers);
        for worker in 0..workers {
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            receivers.push(receiver);
            scope.spawn(move || {
                for path in paths.iter().skip(worker).step_by(workers) {
                    if sender.send(observe(root, path.as_ref(), max)).is_err() {
                        break;
                    }
                }
            });
        }
        let mut batch = Vec::new();
        let mut bytes = 0;
        for (index, path) in paths.iter().enumerate() {
            let observed = if workers == 0 {
                observe(root, path.as_ref(), max)
            } else {
                receivers[index % workers]
                    .recv()
                    .map_err(|_| "A checkpoint read worker stopped")?
            };
            if let Observation::File(content, _) = &observed.kind {
                bytes += content.len() as u64;
            }
            batch.push(observed);
            if bytes >= BATCH_BYTES || batch.len() >= BATCH_PATHS {
                consume(std::mem::take(&mut batch))?;
                bytes = 0;
            }
        }
        if !batch.is_empty() {
            consume(batch)?;
        }
        Ok(())
    })
}

/// A file this process just replaced with `bytes`, described by its metadata
/// without reading it back. `None` when the metadata disagrees.
pub fn written(root: &Path, relative: &str, bytes: Arc<[u8]>) -> Option<Observed> {
    let metadata = fs::symlink_metadata(super::identity::absolute(root, relative)).ok()?;
    let matches = metadata.file_type().is_file() && metadata.len() == bytes.len() as u64;
    matches.then(|| Observed {
        path: relative.to_string(),
        kind: Observation::File(bytes, Mode::of(&metadata)),
        stat: Some(Stat::of(&metadata)),
    })
}

fn read_bounded(path: &Path, limit: u64) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= limit).then_some(bytes))
}

/// The state a recording would produce, comparing bytes with the archive.
fn unchanged(store: &Store, index: &Index, observed: &Observed) -> bool {
    let Some(current) = index.current(&observed.path) else {
        return matches!(observed.kind, Observation::Absent);
    };
    match (&current.state, &observed.kind) {
        (State::Absent, Observation::Absent) => true,
        (
            State::File {
                object,
                length,
                digest,
                mode,
            },
            Observation::File(bytes, observed_mode),
        ) => {
            *length == bytes.len() as u64
                && mode == observed_mode
                && *digest == super::hash::digest(bytes)
                && store.bytes(*object).is_ok_and(|old| old[..] == bytes[..])
        }
        (State::Directory { mode }, Observation::Directory(observed)) => mode == observed,
        (State::Link { target }, Observation::Link(observed)) => target == observed,
        // Content that is never read changed when its metadata did.
        (State::Oversized { length }, Observation::Oversized(observed_length)) => {
            length == observed_length
                && observed
                    .stat
                    .as_ref()
                    .is_none_or(|stat| index.stats.get(&observed.path) == Some(stat))
        }
        (State::Unreadable { reason }, Observation::Unreadable(observed)) => reason == observed,
        _ => false,
    }
}

pub struct Recorded {
    pub seq: u64,
    pub state: State,
}

/// Records `observed` if it differs from the current version. Returns the
/// new version, or `None` when the archive already holds this state.
pub fn record(
    transaction: &mut Transaction<'_>,
    observed: Observed,
    origin: &str,
    operation: u64,
) -> Result<Option<Recorded>, String> {
    if unchanged(transaction.store, transaction.index(), &observed) {
        if let Some(stat) = observed.stat
            && transaction.index().stats.get(&observed.path) != Some(&stat)
            && transaction.index().current(&observed.path).is_some()
        {
            let seq = transaction.seq();
            transaction.record(Record::Stat {
                seq,
                path: observed.path,
                stat,
            });
        }
        return Ok(None);
    }
    let previous = transaction.index().current(&observed.path);
    let discovery = match previous {
        Some(_) => Discovery::Known,
        None if transaction.index().coverage_complete() => Discovery::Created,
        None => Discovery::Found,
    };
    let base = previous.and_then(|version| match version.state {
        State::File { object, .. } => Some(object),
        _ => None,
    });
    let state = match observed.kind {
        Observation::Absent => State::Absent,
        Observation::File(bytes, mode) => {
            let length = bytes.len() as u64;
            let digest = super::hash::digest(&bytes);
            transaction.store.counters.files_read += 1;
            transaction.store.counters.bytes_read += length;
            State::File {
                object: transaction.object(bytes, base)?,
                length,
                digest,
                mode,
            }
        }
        Observation::Directory(mode) => State::Directory { mode },
        Observation::Link(target) => State::Link { target },
        Observation::Oversized(length) => State::Oversized { length },
        Observation::Unreadable(reason) => State::Unreadable { reason },
    };
    let seq = transaction.seq();
    transaction.record(Record::Version(Version {
        seq,
        path: observed.path,
        state: state.clone(),
        stat: observed.stat,
        time: crate::sessions::now(),
        origin: origin.into(),
        operation,
        discovery,
    }));
    Ok(Some(Recorded { seq, state }))
}

/// Records the file bytes already archived by a mutation's durable Prepare.
/// `written` supplies the mode and stat after the tool's replacement. This
/// avoids reconstructing and hashing the same intended object again. The
/// mutation queues a later disk reread because another writer can still race
/// the replacement, including with content of the same length.
pub fn record_prepared(
    transaction: &mut Transaction<'_>,
    written: Observed,
    prepared: &State,
    origin: &str,
    operation: u64,
) -> Result<Option<Recorded>, String> {
    let (
        Observation::File(_, mode),
        State::File {
            object,
            length,
            digest,
            ..
        },
    ) = (written.kind, prepared)
    else {
        return Err("The prepared checkpoint is not a file".into());
    };
    let state = State::File {
        object: *object,
        length: *length,
        digest: *digest,
        mode,
    };
    let current = transaction.index().current(&written.path);
    if current.is_some_and(|version| version.state == state) {
        if let Some(stat) = written.stat
            && transaction.index().stats.get(&written.path) != Some(&stat)
        {
            let seq = transaction.seq();
            transaction.record(Record::Stat {
                seq,
                path: written.path,
                stat,
            });
        }
        return Ok(None);
    }
    let discovery = match current {
        Some(_) => Discovery::Known,
        None if transaction.index().coverage_complete() => Discovery::Created,
        None => Discovery::Found,
    };
    let seq = transaction.seq();
    transaction.record(Record::Version(Version {
        seq,
        path: written.path,
        state: state.clone(),
        stat: written.stat,
        time: crate::sessions::now(),
        origin: origin.into(),
        operation,
        discovery,
    }));
    Ok(Some(Recorded { seq, state }))
}

/// Whether a scan entry's metadata matches what the archive last saw. On
/// Unix the change time moves with every content write, so a preserved
/// modification time cannot hide one; Windows relies on notifications and
/// content verification instead. An oversized file is due once the limit
/// admits it.
pub fn fresh(index: &Index, path: &str, stat: &Stat, directory: bool, max: u64) -> bool {
    let Some(current) = index.current(path) else {
        return false;
    };
    match current.state {
        State::Directory { mode } => directory && mode.0 == stat.mode,
        State::Unreadable { .. } | State::Absent => false,
        State::Oversized { .. } if stat.length <= max => false,
        _ => !directory && index.stats.get(path) == Some(stat),
    }
}
