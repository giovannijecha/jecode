//! Small-object reuse and bounded delta admission.
use super::{CHUNK_BYTES, Transaction};
use crate::checkpoints::frame::{Encoding, HEADER, ObjectMeta};
use crate::checkpoints::hash;
use crate::checkpoints::index::MAX_DEPTH;
use std::sync::Arc;

impl Transaction<'_> {
    /// Stores bytes once, as a delta of `base` when that saves at least half.
    /// Equal digests only nominate reuse; the bytes are compared first.
    pub fn object(&mut self, bytes: Arc<[u8]>, base: Option<u64>) -> Result<u64, String> {
        if bytes.len() > CHUNK_BYTES {
            return self
                .object_reader(
                    std::io::Cursor::new(bytes.as_ref()),
                    bytes.len() as u64,
                    base,
                )
                .map(|(id, _)| id);
        }
        let digest = hash::digest(&bytes);
        let key = (digest, bytes.len() as u64);
        if let Some(candidates) = self.store.index.contents.get(&key) {
            for id in candidates.clone() {
                if self
                    .store
                    .index
                    .objects
                    .get(&id)
                    .is_some_and(|object| matches!(object.meta.encoding, Encoding::Chunks { .. }))
                {
                    continue;
                }
                if self.store.bytes(id).is_ok_and(|old| old[..] == bytes[..]) {
                    self.store.counters.objects_reused += 1;
                    return Ok(id);
                }
            }
        }
        if let Some(candidates) = self.fresh.get(&key) {
            for (id, old) in candidates {
                if old[..] == bytes[..] {
                    self.store.counters.objects_reused += 1;
                    return Ok(*id);
                }
            }
        }
        let id = self.seq();
        let delta = base.and_then(|base| self.delta(base, &bytes));
        let (encoding, data) = match delta {
            Some((encoding, range)) => {
                self.store.counters.objects_delta += 1;
                (encoding, &bytes[range])
            }
            None => {
                self.store.counters.objects_full += 1;
                (Encoding::Full, &bytes[..])
            }
        };
        let meta = ObjectMeta {
            id,
            digest,
            length: bytes.len() as u64,
            encoding,
        };
        let frame = meta.encode(data);
        let checksum = u64::from_le_bytes(frame[16..24].try_into().unwrap_or_default());
        self.objects.push((
            meta,
            self.buffer.len(),
            (frame.len() - HEADER) as u64,
            checksum,
            Vec::new(),
        ));
        self.buffer.extend_from_slice(&frame);
        self.fresh.entry(key).or_default().push((id, bytes.clone()));
        self.store.remember(id, bytes);
        Ok(id)
    }

    fn delta(&self, base: u64, bytes: &[u8]) -> Option<(Encoding, std::ops::Range<usize>)> {
        let object = self.store.index.objects.get(&base)?;
        if object.meta.length > CHUNK_BYTES as u64
            || matches!(object.meta.encoding, Encoding::Chunks { .. })
        {
            return None;
        }
        let depth = object.depth;
        if depth >= MAX_DEPTH || bytes.len() < 256 {
            return None;
        }
        let old = self.store.bytes(base).ok()?;
        let limit = old.len().min(bytes.len());
        let prefix = old
            .iter()
            .zip(bytes.iter())
            .take_while(|(a, b)| a == b)
            .count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(bytes[prefix..].iter().rev())
            .take(limit - prefix)
            .take_while(|(a, b)| a == b)
            .count();
        let literal = bytes.len() - prefix - suffix;
        (literal + 64 <= bytes.len() / 2).then_some((
            Encoding::Delta {
                base,
                prefix: prefix as u64,
                suffix: suffix as u64,
            },
            prefix..bytes.len() - suffix,
        ))
    }
}
