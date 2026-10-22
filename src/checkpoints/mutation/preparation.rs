//! Durable preparation of byte slices and streamed replacement files.
use super::*;
use crate::checkpoints::capture::Observation;
use crate::checkpoints::files::Mode;
use crate::checkpoints::record::{Discovery, Prepare, Version};
use std::sync::Arc;

enum Intended<'a> {
    Bytes(&'a [u8]),
    Reader(&'a mut dyn std::io::Read, u64),
}

impl Intended<'_> {
    fn length(&self) -> u64 {
        match self {
            Self::Bytes(bytes) => bytes.len() as u64,
            Self::Reader(_, length) => *length,
        }
    }

    fn archive(
        &mut self,
        transaction: &mut crate::checkpoints::store::Transaction<'_>,
        length: u64,
        base: Option<u64>,
        inline: &Option<Arc<[u8]>>,
    ) -> Result<(u64, crate::checkpoints::hash::Digest), String> {
        if let Some(bytes) = inline {
            return Ok((
                transaction.object(bytes.clone(), base)?,
                crate::checkpoints::hash::digest(bytes),
            ));
        }
        match self {
            Self::Bytes(bytes) => {
                transaction.object_reader(std::io::Cursor::new(*bytes), length, base)
            }
            Self::Reader(reader, _) => transaction.object_reader(&mut **reader, length, base),
        }
    }
}

impl Mutation<'_> {
    /// Durably records the current state and the intended content.
    pub fn prepare(&mut self, content: &[u8]) {
        self.prepare_inner(Intended::Bytes(content), None);
    }

    /// Saves both sides before installing an already staged replacement.
    #[cfg(test)]
    pub fn prepare_file(&mut self, path: &Path) {
        let result = std::fs::File::open(path)
            .and_then(|file| file.metadata().map(|metadata| (file, metadata.len())));
        match result {
            Ok((file, length)) => self.prepare_reader(file, length),
            Err(error) => self.protection = Protection::Unprotected(error.to_string()),
        }
    }

    /// Archives an intended replacement directly from its source stream.
    pub fn prepare_reader(&mut self, mut reader: impl std::io::Read, length: u64) {
        self.prepare_inner(Intended::Reader(&mut reader, length), None);
    }

    /// Returns the previous text from the same exact read used for the
    /// preimage. A missing path is `Some(None)`; an unreadable, binary or
    /// oversized path yields `None` so the tool can use its normal fallback.
    pub fn prepare_with_previous(
        &mut self,
        content: &[u8],
        max_text_bytes: usize,
    ) -> Option<Option<String>> {
        self.prepare_inner(Intended::Bytes(content), Some(max_text_bytes))
    }

    fn prepare_inner(
        &mut self,
        mut content: Intended<'_>,
        max_text_bytes: Option<usize>,
    ) -> Option<Option<String>> {
        let (Some(path), Some(_)) = (&self.path, &self.lease) else {
            return None;
        };
        if self.prepared.is_some() {
            return None;
        }
        let length = content.length();
        let workspace = self.workspace;
        let mut store = match workspace.store_cancellable(&self.cancellation) {
            Ok(store) => store,
            Err(error) => {
                self.protection = Protection::Unprotected(error);
                return None;
            }
        };
        let max = store.policy.max_file_bytes;
        let observed = capture::observe(&workspace.root, path, max);
        let previous = max_text_bytes
            .filter(|limit| length <= *limit as u64)
            .and_then(|limit| match &observed.kind {
                Observation::Absent => Some(None),
                Observation::File(bytes, _) if bytes.len() <= limit && !bytes.contains(&0) => {
                    std::str::from_utf8(bytes)
                        .ok()
                        .map(|text| Some(text.to_owned()))
                }
                _ => None,
            });
        let absent = matches!(observed.kind, Observation::Absent);
        let owner = store.owner.clone();
        let bytes: Option<Arc<[u8]>> = match &content {
            Intended::Bytes(content) if length <= max.min(capture::INLINE_BYTES) => {
                Some((*content).into())
            }
            _ => None,
        };
        let result = store.transaction_cancellable(true, &self.cancellation, |transaction| {
            let (group, snapshot) =
                crate::checkpoints::recovery::begin_snapshot(transaction, &self.origin, self.kind)?;
            transaction.flush(false)?;
            capture::record(transaction, observed, "external", 0)?;
            if absent && transaction.index().current(path).is_none() {
                // Make the absence explicit so restoring it removes the file.
                let seq = transaction.seq();
                let discovery = if transaction.index().coverage_complete() {
                    Discovery::Created
                } else {
                    Discovery::Found
                };
                transaction.record(Record::Version(Version {
                    seq,
                    path: path.clone(),
                    state: State::Absent,
                    stat: None,
                    time: crate::checkpoints::clock::now(),
                    origin: "external".into(),
                    operation: 0,
                    discovery,
                }));
            }
            // Apply what was observed so the index shows the actual state.
            transaction.flush(false)?;
            let before = transaction.index().current_state(path);
            if before.recoverable() {
                crate::checkpoints::recovery::add(transaction, group, snapshot, path, &before)?;
            }
            let after = if length <= max {
                let base = match before {
                    State::File { object, .. } => Some(object),
                    _ => None,
                };
                let (object, digest) = content.archive(transaction, length, base, &bytes)?;
                State::File {
                    object,
                    length,
                    digest,
                    mode: match before {
                        State::File { mode, .. } => mode,
                        _ => Mode::default(),
                    },
                }
            } else {
                State::Oversized { length }
            };
            let seq = transaction.seq();
            transaction.record(Record::Prepare(Prepare {
                seq,
                owner: owner.clone(),
                origin: self.origin.clone(),
                kind: self.kind.into(),
                path: path.clone(),
                before: before.clone(),
                after: after.clone(),
                time: crate::checkpoints::clock::now(),
            }));
            Ok((seq, snapshot, before, after))
        });
        match result {
            Ok((seq, snapshot, before, after)) => {
                self.protection = if before.recoverable() {
                    Protection::Protected {
                        before: snapshot,
                        after: None,
                    }
                } else {
                    Protection::Unprotected(format!(
                        "the previous version was not captured ({})",
                        before.label()
                    ))
                };
                self.prepared = Some((seq, before, after));
            }
            Err(error) => self.protection = Protection::Unprotected(error),
        }
        previous
    }
}
