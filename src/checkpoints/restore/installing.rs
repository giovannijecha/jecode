use super::super::files;
use super::*;
use std::fs;
use std::io::Read;

pub(super) fn install(
    entry: &Inspected,
    root: &std::path::Path,
    staged_file: Option<files::PendingFile>,
    staged_link: Option<files::PendingLink>,
) -> Result<(), String> {
    let path = paths::absolute(root, &entry.path);
    files::confined_parent(root, &entry.path)?;
    match &entry.saved {
        State::Absent => files::remove(&path)
            .map_err(|error| format!("Could not remove {}: {error}", entry.path)),
        State::Directory { .. } => {
            if !matches!(entry.current, State::Directory { .. }) {
                files::remove(&path)
                    .map_err(|error| format!("Could not clear {}: {error}", entry.path))?;
                fs::create_dir(&path).map_err(|error| {
                    format!("Could not create directory {}: {error}", entry.path)
                })?;
                #[cfg(unix)]
                files::directory_mode(&path, files::Mode(0o700)).map_err(|error| {
                    format!(
                        "Could not make new directory accessible {}: {error}",
                        entry.path
                    )
                })?;
            }
            Ok(())
        }
        State::File { .. } => {
            let staged = staged_file.ok_or_else(|| format!("No staged file for {}", entry.path))?;
            if matches!(entry.current, State::Directory { .. } | State::Link { .. }) {
                files::remove(&path)
                    .map_err(|error| format!("Could not clear {}: {error}", entry.path))?;
            }
            staged
                .install_deferred()
                .map_err(|error| format!("Could not install {}: {error}", entry.path))
        }
        State::Link { .. } => {
            let staged = staged_link.ok_or_else(|| format!("No staged link for {}", entry.path))?;
            if matches!(entry.current, State::Directory { .. }) {
                files::remove(&path)
                    .map_err(|error| format!("Could not clear {}: {error}", entry.path))?;
            }
            staged
                .install_deferred()
                .map_err(|error| format!("Could not install link {}: {error}", entry.path))
        }
        State::Unreadable { reason } => Err(format!("{} was not captured: {reason}", entry.path)),
    }
}

pub(super) fn stage_link(
    entry: &Inspected,
    root: &std::path::Path,
) -> Result<Option<files::PendingLink>, String> {
    let State::Link { target, directory } = &entry.saved else {
        return Ok(None);
    };
    files::stage_link(target, *directory, &paths::absolute(root, &entry.path))
        .map(Some)
        .map_err(|error| format!("Could not stage link {}: {error}", entry.path))
}

pub(super) fn finalize_directory(
    root: &std::path::Path,
    relative: &str,
    mode: files::Mode,
) -> Result<(), String> {
    files::confined_parent(root, relative)?;
    let path = paths::absolute(root, relative);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("Could not recheck directory {relative}: {error}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!(
            "Directory {relative} changed before mode installation"
        ));
    }
    #[cfg(unix)]
    files::directory_mode(&path, mode)
        .map_err(|error| format!("Could not set directory mode {relative}: {error}"))?;
    #[cfg(not(unix))]
    mode.apply_path(&path)
        .map_err(|error| format!("Could not set directory mode {relative}: {error}"))?;
    Ok(())
}

struct VerifiedReader<R> {
    source: R,
    hasher: Option<hash::Hasher>,
    remaining: u64,
    expected: hash::Digest,
    cancel: Cancellation,
}

impl<R: Read> Read for VerifiedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.requested() {
            return Err(std::io::Error::other("Recovery restore was cancelled"));
        }
        let count = self.source.read(buffer)?;
        if count as u64 > self.remaining {
            return Err(std::io::Error::other(
                "Recovery source exceeded saved length",
            ));
        }
        if count == 0 && self.remaining != 0 {
            return Err(std::io::Error::other("Recovery source is truncated"));
        }
        self.remaining -= count as u64;
        if count != 0 {
            self.hasher
                .as_mut()
                .expect("hasher available")
                .update(&buffer[..count]);
        }
        Ok(count)
    }
}

impl<R> VerifiedReader<R> {
    fn finish(mut self) -> Result<(), String> {
        if self.remaining != 0 {
            return Err("Recovery source is truncated".into());
        }
        if self.hasher.take().expect("hasher available").finish() != self.expected {
            return Err("Recovery source failed its content checksum".into());
        }
        Ok(())
    }
}

pub(super) fn stage_file(
    loaded: &super::super::store::Loaded,
    entry: &Inspected,
    root: &std::path::Path,
    cancel: &Cancellation,
) -> Result<Option<files::PendingFile>, String> {
    let State::File {
        length,
        digest,
        mode,
        ..
    } = &entry.saved
    else {
        return Ok(None);
    };
    let mut source = VerifiedReader {
        source: loaded.reader(&entry.saved)?,
        hasher: Some(hash::Hasher::new(*length)),
        remaining: *length,
        expected: *digest,
        cancel: cancel.clone(),
    };
    let staged = files::stage_reader(&paths::absolute(root, &entry.path), &mut source, *mode)
        .map_err(|error| format!("Could not stage {}: {error}", entry.path))?;
    source.finish()?;
    Ok(Some(staged))
}

pub(super) fn order(entries: &mut [Inspected]) {
    entries.sort_by(|a, b| {
        let depth = |entry: &Inspected| entry.path.bytes().filter(|byte| *byte == b'/').count();
        let rank = |entry: &Inspected| match &entry.saved {
            State::Absent => 0,
            State::Directory { .. } if !matches!(entry.current, State::Directory { .. }) => 1,
            State::Directory { .. } => 3,
            _ => 2,
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| match rank(a) {
                0 | 3 => depth(b).cmp(&depth(a)),
                _ => depth(a).cmp(&depth(b)),
            })
            .then_with(|| a.path.cmp(&b.path))
    });
}
