//! Chunked object admission without retaining a whole large file in memory.
use super::{CHUNK_BYTES, Transaction};
use crate::checkpoints::frame::{Encoding, HEADER, MANIFEST_CHILDREN, ObjectMeta};
use crate::checkpoints::hash::{Digest, Hasher};
use std::io::Read;
use std::sync::Arc;

struct Level {
    span: u64,
    start: u64,
    hasher: Hasher,
    children: Vec<u64>,
}

impl Level {
    fn new(span: u64, length: u64) -> Self {
        Self {
            span,
            start: 0,
            hasher: Hasher::new(span.min(length)),
            children: Vec::with_capacity(MANIFEST_CHILDREN),
        }
    }

    fn end(&self, length: u64) -> u64 {
        self.start + self.span.min(length - self.start)
    }
}

impl Transaction<'_> {
    /// Stores exactly `length` bytes. Every chunk can stand alone if the read
    /// or final commit fails; only the manifest makes the file state visible.
    pub fn object_reader(
        &mut self,
        mut reader: impl Read,
        length: u64,
        base: Option<u64>,
    ) -> Result<(u64, Digest), String> {
        if length <= CHUNK_BYTES as u64 {
            let mut bytes = vec![0; length as usize];
            reader
                .read_exact(&mut bytes)
                .map_err(|error| format!("Could not read checkpoint file: {error}"))?;
            let mut extra = [0];
            if reader
                .read(&mut extra)
                .map_err(|error| format!("Could not read checkpoint file: {error}"))?
                != 0
            {
                return Err("Checkpoint file grew during capture".into());
            }
            let digest = crate::checkpoints::hash::digest(&bytes);
            return self.object(Arc::from(bytes), base).map(|id| (id, digest));
        }
        let mut levels = Vec::new();
        let mut span = (CHUNK_BYTES * MANIFEST_CHILDREN) as u64;
        loop {
            levels.push(Level::new(span, length));
            if span >= length {
                break;
            }
            span = span.saturating_mul(MANIFEST_CHILDREN as u64);
        }
        let mut remaining = length;
        let mut position = 0u64;
        let mut root = None;
        while remaining != 0 {
            let size = remaining.min(CHUNK_BYTES as u64) as usize;
            let mut bytes = vec![0; size];
            reader
                .read_exact(&mut bytes)
                .map_err(|error| format!("Could not read checkpoint file: {error}"))?;
            for level in &mut levels {
                level.hasher.update(&bytes);
            }
            let mut child = self.object(Arc::from(bytes), None)?;
            // The next chunk must never enlarge one buffered journal append to
            // a whole large file. Orphan chunks are harmless after failure.
            self.flush(false)?;
            remaining -= size as u64;
            position += size as u64;
            for level in &mut levels {
                level.children.push(child);
                if position != level.end(length) {
                    break;
                }
                let group_length = position - level.start;
                let next_length = if position < length {
                    level.span.min(length - position)
                } else {
                    0
                };
                let hasher = std::mem::replace(&mut level.hasher, Hasher::new(next_length));
                child = self.manifest(
                    group_length,
                    hasher.finish(),
                    &std::mem::take(&mut level.children),
                )?;
                self.flush(false)?;
                level.start = position;
                if position == length {
                    root = Some(child);
                }
            }
        }
        let mut extra = [0];
        if reader
            .read(&mut extra)
            .map_err(|error| format!("Could not read checkpoint file: {error}"))?
            != 0
        {
            return Err("Checkpoint file grew during capture".into());
        }
        let id = root.ok_or("Checkpoint root manifest is missing")?;
        let digest = self
            .store
            .index
            .objects
            .get(&id)
            .ok_or("Checkpoint root manifest was not recorded")?
            .meta
            .digest;
        Ok((id, digest))
    }

    fn manifest(&mut self, length: u64, digest: Digest, chunks: &[u64]) -> Result<u64, String> {
        if chunks.is_empty() || chunks.len() > MANIFEST_CHILDREN {
            return Err("Checkpoint manifest has an invalid child count".into());
        }
        let count = chunks.len() as u64;
        let id = self.seq();
        let meta = ObjectMeta {
            id,
            digest,
            length,
            encoding: Encoding::Chunks { count },
        };
        let mut data = Vec::with_capacity(chunks.len() * 8);
        for chunk in chunks {
            data.extend_from_slice(&chunk.to_le_bytes());
        }
        let frame = meta.encode(&data);
        let checksum = u64::from_le_bytes(frame[16..24].try_into().unwrap_or_default());
        self.objects.push((
            meta,
            self.buffer.len(),
            (frame.len() - HEADER) as u64,
            checksum,
            chunks.to_vec(),
        ));
        self.buffer.extend_from_slice(&frame);
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoints::gc;
    use crate::checkpoints::identity;
    use crate::checkpoints::record::{Discovery, Record, State, Version};
    use crate::checkpoints::store::Store;
    use crate::test_support::Directory;
    use std::io::{Cursor, Read, Seek, SeekFrom};

    #[test]
    fn chunked_file_survives_reopen_and_compaction_with_exact_reads() {
        let home = Directory::new();
        let root = Directory::new();
        let location = identity::locate(home.path(), root.path(), true)
            .unwrap()
            .unwrap();
        let bytes: Vec<u8> = (0..(3 * CHUNK_BYTES + 317))
            .map(|index| ((index * 37) ^ (index >> 8)) as u8)
            .collect();
        let mut store = Store::open(location.clone()).unwrap();
        let (id, digest) = store
            .transaction(true, |transaction| {
                let (id, digest) =
                    transaction.object_reader(Cursor::new(&bytes), bytes.len() as u64, None)?;
                let seq = transaction.seq();
                transaction.record(Record::Version(Version {
                    seq,
                    path: "large.bin".into(),
                    state: State::File {
                        object: id,
                        length: bytes.len() as u64,
                        digest,
                        mode: crate::checkpoints::files::Mode(0),
                    },
                    stat: None,
                    time: 0,
                    origin: "test".into(),
                    operation: 0,
                    discovery: Discovery::Known,
                }));
                Ok((id, digest))
            })
            .unwrap();
        assert!(matches!(
            store.index.objects.get(&id).unwrap().meta.encoding,
            Encoding::Chunks { .. }
        ));
        assert_eq!(digest, crate::checkpoints::hash::digest(&bytes));
        assert!(store.equal_reader(id, Cursor::new(&bytes)).unwrap());
        assert!(
            !store
                .equal_reader(id, Cursor::new(&bytes[..bytes.len() - 1]))
                .unwrap()
        );
        let mut reader = store.reader(id).unwrap();
        reader
            .seek(SeekFrom::Start(CHUNK_BYTES as u64 - 7))
            .unwrap();
        let mut tail = Vec::new();
        reader.read_to_end(&mut tail).unwrap();
        assert_eq!(tail.as_slice(), &bytes[CHUNK_BYTES - 7..]);
        drop(store);

        let collection = gc::collect(&location, true).unwrap().unwrap();
        assert_eq!(collection.generation, 2);
        let mut store = Store::open(location).unwrap();
        assert!(store.equal_reader(id, Cursor::new(&bytes)).unwrap());
        store.index.objects.get_mut(&id).unwrap().meta.digest.0[0] ^= 1;
        let mut reader = store.reader(id).unwrap();
        assert!(reader.read_to_end(&mut Vec::new()).is_err());
    }

    #[test]
    fn nested_manifests_replay_and_keep_descendants_during_gc() {
        let home = Directory::new();
        let root = Directory::new();
        let location = identity::locate(home.path(), root.path(), true)
            .unwrap()
            .unwrap();
        let bytes: Vec<u8> = (0..(2 * CHUNK_BYTES))
            .map(|index| ((index * 23) ^ (index >> 11)) as u8)
            .collect();
        let mut store = Store::open(location.clone()).unwrap();
        let id = store
            .transaction(true, |transaction| {
                let mut leaves = Vec::new();
                for group in bytes.chunks(CHUNK_BYTES) {
                    let mut chunks = Vec::new();
                    for part in group.chunks(CHUNK_BYTES / 2) {
                        chunks.push(transaction.object(Arc::from(part), None)?);
                    }
                    leaves.push(transaction.manifest(
                        group.len() as u64,
                        crate::checkpoints::hash::digest(group),
                        &chunks,
                    )?);
                }
                let id = transaction.manifest(
                    bytes.len() as u64,
                    crate::checkpoints::hash::digest(&bytes),
                    &leaves,
                )?;
                let seq = transaction.seq();
                transaction.record(Record::Version(Version {
                    seq,
                    path: "nested.bin".into(),
                    state: State::File {
                        object: id,
                        length: bytes.len() as u64,
                        digest: crate::checkpoints::hash::digest(&bytes),
                        mode: crate::checkpoints::files::Mode(0),
                    },
                    stat: None,
                    time: 0,
                    origin: "test".into(),
                    operation: 0,
                    discovery: Discovery::Known,
                }));
                Ok(id)
            })
            .unwrap();
        assert!(store.equal_reader(id, Cursor::new(&bytes)).unwrap());
        drop(store);
        gc::collect(&location, true).unwrap().unwrap();
        let mut store = Store::open(location).unwrap();
        let first_manifest = store.index.manifests[&id][0];
        let first_chunk = store.index.manifests[&first_manifest][0];
        store
            .index
            .objects
            .get_mut(&first_chunk)
            .unwrap()
            .meta
            .digest
            .0[0] ^= 1;
        assert!(store.equal_reader(id, Cursor::new(&bytes)).is_err());
        store
            .index
            .objects
            .get_mut(&first_chunk)
            .unwrap()
            .meta
            .digest
            .0[0] ^= 1;
        assert!(store.equal_reader(id, Cursor::new(&bytes)).unwrap());
        store.index.manifests.insert(id, vec![id]);
        store.index.objects.get_mut(&id).unwrap().meta.encoding = Encoding::Chunks { count: 1 };
        assert!(store.reader(id).unwrap().read(&mut [0u8; 1]).is_err());
    }

    #[test]
    fn short_stream_never_publishes_a_manifest() {
        let home = Directory::new();
        let root = Directory::new();
        let location = identity::locate(home.path(), root.path(), true)
            .unwrap()
            .unwrap();
        let mut store = Store::open(location.clone()).unwrap();
        let bytes = vec![9u8; CHUNK_BYTES + 7];
        let error = store
            .transaction(true, |transaction| {
                transaction.object_reader(Cursor::new(&bytes), bytes.len() as u64 + 1, None)
            })
            .unwrap_err();
        assert!(error.contains("Could not read checkpoint file"));
        assert!(store.index.manifests.is_empty());
        drop(store);
        let store = Store::open(location).unwrap();
        assert!(store.index.manifests.is_empty());
    }
}
