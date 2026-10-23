//! Stream before bytes into one unpublished pack, then publish once synced.
use super::files::{self, Mode};
use super::format;
use super::hash;
use super::locks;
use super::paths;
use super::state::State;
use super::store::Store;
use super::types::{Capture, Issue, Snapshot};
use crate::cancel::Cancellation;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const COPY_BUFFER: usize = 256 * 1024;

#[cfg(test)]
type CaptureHook = Box<dyn FnOnce(&File)>;

#[cfg(test)]
thread_local! {
    pub(super) static BEFORE_RELEASE: std::cell::RefCell<Option<CaptureHook>> = std::cell::RefCell::new(None);
}

struct CaptureRequest<'a> {
    reference: &'a str,
    created: u64,
    tool: &'a str,
    origin: &'a str,
    selection: &'a paths::Selection,
}

impl Store {
    pub fn capture(
        &self,
        tool: &str,
        origin: &str,
        requested: &[String],
        cancel: &Cancellation,
    ) -> Result<Capture, String> {
        check(cancel)?;
        self.capture_selected(
            tool,
            origin,
            paths::discover(&self.root, requested, &self.excluded, cancel),
            cancel,
        )
        .map(|(capture, _)| capture)
    }

    /// Saves exactly the named entries and returns the published snapshot, so
    /// a restore can guard its undo without reopening the new pack.
    pub(super) fn capture_exact(
        &self,
        tool: &str,
        origin: &str,
        requested: &[String],
        cancel: &Cancellation,
    ) -> Result<(Capture, Option<Snapshot>), String> {
        check(cancel)?;
        self.capture_selected(
            tool,
            origin,
            paths::discover_exact(&self.root, requested, &self.excluded, cancel),
            cancel,
        )
    }

    fn capture_selected(
        &self,
        tool: &str,
        origin: &str,
        selection: Result<paths::Selection, String>,
        cancel: &Cancellation,
    ) -> Result<(Capture, Option<Snapshot>), String> {
        let selection = match selection {
            Ok(selection) => selection,
            Err(_) if cancel.requested() => return Err("Operation cancelled".into()),
            Err(error) => return Ok((Capture::failed(vec![(String::new(), error)]), None)),
        };
        let mut partial = selection.partial.clone();
        if selection.paths.is_empty() && selection.roots.is_empty() {
            return Ok((Capture::failed(partial), None));
        }
        let created = super::clock::now();
        let (reference, temporary, file) = loop {
            let reference = format!(
                "rc-{created:016x}-{:08x}-{:016x}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let temporary = self.directory.join(format!(".{reference}.pending"));
            match crate::disk::private_options()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&temporary)
            {
                Ok(file) => break (reference, temporary, file),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    partial.push((
                        String::new(),
                        format!("Could not create recovery pack: {error}"),
                    ));
                    return Ok((Capture::failed(partial), None));
                }
            }
        };
        let mut writer = match locks::Exclusive::from_file(file) {
            Ok(writer) => writer,
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                partial.push((
                    String::new(),
                    format!("Could not lock recovery pack: {error}"),
                ));
                return Ok((Capture::failed(partial), None));
            }
        };
        let request = CaptureRequest {
            reference: &reference,
            created,
            tool,
            origin,
            selection: &selection,
        };
        let result = self.capture_into(writer.file_mut(), &request, &mut partial, cancel);
        #[cfg(test)]
        BEFORE_RELEASE.with(|hook| {
            if let Some(action) = hook.borrow_mut().take() {
                action(writer.file_mut());
            }
        });
        drop(writer);
        let result = result.and_then(|saved| {
            self.publish(&reference, cancel)?;
            Ok(saved)
        });
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        match result {
            Ok((saved, snapshot)) => Ok((
                Capture {
                    reference: Some(reference),
                    created,
                    saved,
                    partial,
                },
                Some(snapshot),
            )),
            Err(error) if error == "Operation cancelled" => Err(error),
            Err(error) => {
                partial.push((String::new(), error));
                Ok((Capture::failed(partial), None))
            }
        }
    }

    fn capture_into(
        &self,
        file: &mut File,
        request: &CaptureRequest<'_>,
        partial: &mut Vec<Issue>,
        cancel: &Cancellation,
    ) -> Result<(usize, Snapshot), String> {
        self.claim(format::HEADER, cancel)?;
        format::header(file, request.created)?;
        let mut entries = BTreeMap::new();
        let mut buffer = vec![0u8; COPY_BUFFER];
        for path in &request.selection.paths {
            check(cancel)?;
            if blocked_ancestor(path, &entries) {
                entries.insert(path.clone(), State::Absent);
                continue;
            }
            let at = file.stream_position().map_err(|e| e.to_string())?;
            match self.capture_one(file, path, &mut buffer, cancel) {
                Ok(state) => {
                    entries.insert(path.clone(), state);
                }
                Err(_) if cancel.requested() => return Err("Operation cancelled".into()),
                Err(error) => {
                    file.set_len(at).map_err(|e| e.to_string())?;
                    file.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
                    partial.push((path.clone(), error.clone()));
                    entries.insert(path.clone(), State::Unreadable { reason: error });
                }
            }
        }
        let mut roots = Vec::new();
        for root in &request.selection.roots {
            check(cancel)?;
            if partial.iter().any(|(path, _)| within_scope(root, path)) {
                continue;
            }
            match paths::discover(
                &self.root,
                std::slice::from_ref(root),
                &self.excluded,
                cancel,
            ) {
                Ok(recheck) => {
                    let before: Vec<_> = request
                        .selection
                        .paths
                        .iter()
                        .filter(|p| within_scope(root, p))
                        .collect();
                    let after: Vec<_> = recheck
                        .paths
                        .iter()
                        .filter(|p| within_scope(root, p))
                        .collect();
                    if recheck.roots.contains(root) && recheck.partial.is_empty() && before == after
                    {
                        roots.push(root.clone());
                    } else {
                        partial.push((
                            root.clone(),
                            "Directory membership changed during capture".into(),
                        ));
                    }
                }
                Err(_) if cancel.requested() => return Err("Operation cancelled".into()),
                Err(error) => partial.push((
                    root.clone(),
                    format!("Could not verify directory scope: {error}"),
                )),
            }
        }
        let saved = entries.values().filter(|state| state.recoverable()).count();
        if saved == 0 && roots.is_empty() {
            return Err("No recoverable before state was saved".into());
        }
        let snapshot = Snapshot {
            reference: request.reference.into(),
            created: request.created,
            tool: request.tool.into(),
            origin: request.origin.into(),
            entries,
            roots,
            excluded: self.excluded.clone(),
            ignored: request.selection.ignored.clone(),
            ignore_rules: request.selection.ignore_rules.clone(),
            partial: partial.clone(),
        };
        let metadata = format::encode(&snapshot)?;
        self.claim(metadata.len() as u64 + 24, cancel)?;
        format::finish(file, &metadata)?;
        check(cancel)?;
        file.sync_all()
            .map_err(|e| format!("Could not sync recovery pack: {e}"))?;
        check(cancel)?;
        Ok((saved, snapshot))
    }

    fn capture_one(
        &self,
        pack: &mut File,
        relative: &str,
        buffer: &mut [u8],
        cancel: &Cancellation,
    ) -> Result<State, String> {
        let path = paths::absolute(&self.root, relative);
        files::confined_parent(&self.root, relative)?;
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(State::Absent),
            Err(error) => return Err(format!("Could not inspect {relative}: {error}")),
        };
        if metadata.file_type().is_symlink() {
            #[cfg(windows)]
            let directory = {
                use std::os::windows::fs::FileTypeExt;
                metadata.file_type().is_symlink_dir()
            };
            #[cfg(not(windows))]
            let directory = false;
            let target =
                fs::read_link(&path).map_err(|e| format!("Could not read link {relative}: {e}"))?;
            let target = target
                .to_str()
                .ok_or_else(|| format!("Non-UTF-8 link target at {relative}"))?;
            return Ok(State::Link {
                target: target.to_owned(),
                directory,
            });
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(format!("Unsupported reparse point at {relative}"));
            }
        }
        if metadata.is_dir() {
            return Ok(State::Directory {
                mode: Mode::of(&metadata),
            });
        }
        if !metadata.is_file() {
            return Err(format!("Unsupported file type at {relative}"));
        }
        let length = metadata.len();
        self.claim(length, cancel)?;
        let mut source =
            File::open(&path).map_err(|e| format!("Could not open {relative}: {e}"))?;
        let opened = source.metadata().map_err(|e| e.to_string())?;
        if !opened.is_file() || opened.len() != length {
            return Err(format!("{relative} changed during capture"));
        }
        let offset = pack.stream_position().map_err(|e| e.to_string())?;
        let mut hasher = hash::Hasher::new(length);
        let mut remaining = length;
        while remaining != 0 {
            check(cancel)?;
            let count = buffer.len().min(remaining as usize);
            let n = source
                .read(&mut buffer[..count])
                .map_err(|e| format!("Could not read {relative}: {e}"))?;
            if n == 0 {
                return Err(format!("{relative} shortened during capture"));
            }
            pack.write_all(&buffer[..n])
                .map_err(|e| format!("Could not save {relative}: {e}"))?;
            #[cfg(test)]
            if self.cancel_after_chunk.load(Ordering::Relaxed) {
                cancel.cancel();
            }
            hasher.update(&buffer[..n]);
            remaining -= n as u64;
        }
        check(cancel)?;
        let after = fs::symlink_metadata(&path)
            .map_err(|e| format!("Could not recheck {relative}: {e}"))?;
        let source_after = source.metadata().map_err(|e| e.to_string())?;
        if !after.is_file()
            || after.len() != length
            || source_after.len() != length
            || metadata.modified().ok() != after.modified().ok()
            || metadata.modified().ok() != source_after.modified().ok()
        {
            return Err(format!("{relative} changed during capture"));
        }
        Ok(State::File {
            offset,
            length,
            digest: hasher.finish(),
            mode: Mode::of(&metadata),
        })
    }

    pub(super) fn publish(&self, reference: &str, cancel: &Cancellation) -> Result<(), String> {
        let created =
            u64::from_str_radix(&reference[3..19], 16).map_err(|_| "Invalid recovery reference")?;
        if super::clock::now() >= created.saturating_add(super::types::TTL_MILLIS) {
            return Err("Recovery expired during capture".into());
        }
        let _coordination = locks::Exclusive::acquire_cancellable(
            &self.directory.join(".internal/identity.lock"),
            cancel,
        )
        .map_err(|e| e.to_string())?;
        let target = self.directory.join(format!("{reference}.rcp"));
        if fs::symlink_metadata(&target).is_ok() {
            return Err("Recovery reference collision".into());
        }
        let source = self.directory.join(format!(".{reference}.pending"));
        check(cancel)?;
        fs::rename(&source, &target).map_err(|e| format!("Could not publish recovery pack: {e}"))
    }
}

fn within_scope(root: &str, path: &str) -> bool {
    root.is_empty()
        || root == path
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn blocked_ancestor(path: &str, entries: &BTreeMap<String, State>) -> bool {
    path.match_indices('/').any(|(at, _)| {
        entries.get(&path[..at]).is_some_and(|state| {
            matches!(
                state,
                State::Absent | State::File { .. } | State::Link { .. }
            )
        })
    })
}

fn check(cancel: &Cancellation) -> Result<(), String> {
    if cancel.requested() {
        Err("Operation cancelled".into())
    } else {
        Ok(())
    }
}
