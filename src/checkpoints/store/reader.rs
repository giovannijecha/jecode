//! Bounded reads of archived files. Legacy objects retain their original
//! reconstruction path; chunked files keep only one verified chunk in memory.
use super::Store;
use crate::checkpoints::frame::{Encoding, MANIFEST_CHILDREN};
use crate::checkpoints::hash::{Digest, Hasher};
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::sync::Arc;

pub struct ObjectReader<'a> {
    inner: ReaderInner<'a>,
}

enum ReaderInner<'a> {
    Small(Cursor<Arc<[u8]>>),
    Chunks {
        store: &'a Store,
        root: u64,
        length: u64,
        digest: Digest,
        position: u64,
        hasher: Option<Hasher>,
        validated: bool,
        cached: Option<(Chunk, Arc<[u8]>)>,
    },
}

#[derive(Clone, Copy)]
struct Chunk {
    id: u64,
    start: u64,
    length: u64,
}

fn io_error(error: String) -> io::Error {
    io::Error::other(error)
}

fn validate_node(store: &Store, id: u64) -> Result<&[u64], String> {
    let object = store
        .index
        .objects
        .get(&id)
        .ok_or_else(|| format!("Checkpoint object {id} is missing"))?;
    let Encoding::Chunks { count } = object.meta.encoding else {
        return Err(format!("Checkpoint object {id} is not a manifest"));
    };
    let children = store
        .index
        .manifests
        .get(&id)
        .ok_or_else(|| format!("Checkpoint object {id} has no manifest"))?;
    if children.is_empty() || children.len() > MANIFEST_CHILDREN || children.len() as u64 != count {
        return Err(format!("Checkpoint object {id} has an invalid child count"));
    }
    let mut total = 0u64;
    for child in children {
        let part = store
            .index
            .objects
            .get(child)
            .ok_or_else(|| format!("Checkpoint object {id} refers to missing child {child}"))?;
        if part.meta.length == 0 {
            return Err(format!("Checkpoint object {id} has empty child {child}"));
        }
        total = total
            .checked_add(part.meta.length)
            .ok_or_else(|| format!("Checkpoint object {id} is too large"))?;
    }
    if total != object.meta.length {
        return Err(format!("Checkpoint object {id} has an invalid child total"));
    }
    Ok(children)
}

fn locate_leaf(store: &Store, root: u64, offset: u64) -> Result<Chunk, String> {
    let mut id = root;
    let mut start = 0u64;
    let mut ancestors = Vec::new();
    loop {
        if ancestors.len() == 64 || ancestors.contains(&id) {
            return Err(format!(
                "Checkpoint manifest {root} has a cycle or invalid depth"
            ));
        }
        ancestors.push(id);
        let children = validate_node(store, id)?;
        let mut at = start;
        let mut selected = None;
        for child in children {
            let part = store
                .index
                .objects
                .get(child)
                .ok_or("Checkpoint child is missing")?;
            let end = at + part.meta.length;
            if offset < end {
                selected = Some((*child, at, part.meta.length, part.meta.encoding));
                break;
            }
            at = end;
        }
        let (child, child_start, length, encoding) =
            selected.ok_or_else(|| format!("Checkpoint manifest {id} has no child at {offset}"))?;
        if matches!(encoding, Encoding::Chunks { .. }) {
            id = child;
            start = child_start;
        } else {
            if length > super::CHUNK_BYTES as u64 {
                return Err(format!(
                    "Checkpoint manifest {id} has oversized chunk {child}"
                ));
            }
            return Ok(Chunk {
                id: child,
                start: child_start,
                length,
            });
        }
    }
}

impl Store {
    pub fn reader(&self, id: u64) -> Result<ObjectReader<'_>, String> {
        let object = self
            .index
            .objects
            .get(&id)
            .ok_or_else(|| format!("Checkpoint object {id} is not available"))?;
        if !matches!(object.meta.encoding, Encoding::Chunks { .. }) {
            return Ok(ObjectReader {
                inner: ReaderInner::Small(Cursor::new(self.bytes(id)?)),
            });
        }
        validate_node(self, id)?;
        Ok(ObjectReader {
            inner: ReaderInner::Chunks {
                store: self,
                root: id,
                length: object.meta.length,
                digest: object.meta.digest,
                position: 0,
                hasher: Some(Hasher::new(object.meta.length)),
                validated: false,
                cached: None,
            },
        })
    }

    pub fn equal_reader(&self, id: u64, mut other: impl Read) -> Result<bool, String> {
        let mut archived = self.reader(id)?;
        let mut left = [0u8; 64 * 1024];
        let mut right = [0u8; 64 * 1024];
        loop {
            let count = archived
                .read(&mut left)
                .map_err(|error| format!("Could not verify checkpoint object: {error}"))?;
            if count == 0 {
                return other
                    .read(&mut right[..1])
                    .map(|extra| extra == 0)
                    .map_err(|error| format!("Could not compare file with checkpoint: {error}"));
            }
            if let Err(error) = other.read_exact(&mut right[..count]) {
                if error.kind() == io::ErrorKind::UnexpectedEof {
                    return Ok(false);
                }
                return Err(format!("Could not compare file with checkpoint: {error}"));
            }
            if left[..count] != right[..count] {
                return Ok(false);
            }
        }
    }

    pub fn objects_equal(&self, left: u64, right: u64) -> Result<bool, String> {
        if left == right {
            return Ok(true);
        }
        let a = self
            .index
            .objects
            .get(&left)
            .ok_or("Checkpoint object is missing")?;
        let b = self
            .index
            .objects
            .get(&right)
            .ok_or("Checkpoint object is missing")?;
        if a.meta.length != b.meta.length || a.meta.digest != b.meta.digest {
            return Ok(false);
        }
        self.equal_reader(left, self.reader(right)?)
    }
}

impl Read for ObjectReader<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        match &mut self.inner {
            ReaderInner::Small(cursor) => cursor.read(out),
            ReaderInner::Chunks {
                store,
                root,
                length,
                digest,
                position,
                hasher,
                validated,
                cached,
            } => {
                if out.is_empty() {
                    return Ok(0);
                }
                if *position >= *length {
                    verify_root(store, *root, *length, *digest, hasher, validated)?;
                    return Ok(0);
                }
                let mut written = 0;
                while written < out.len() && *position < *length {
                    let part = match cached.as_ref() {
                        Some((chunk, _))
                            if *position >= chunk.start
                                && *position - chunk.start < chunk.length =>
                        {
                            *chunk
                        }
                        _ => {
                            let chunk = locate_leaf(store, *root, *position).map_err(io_error)?;
                            *cached = Some((chunk, store.bytes(chunk.id).map_err(io_error)?));
                            chunk
                        }
                    };
                    let bytes = &cached.as_ref().expect("chunk was loaded").1;
                    if bytes.len() as u64 != part.length {
                        return Err(io_error("Checkpoint chunk length changed".into()));
                    }
                    let offset = (*position - part.start) as usize;
                    let count = (bytes.len() - offset).min(out.len() - written);
                    out[written..written + count].copy_from_slice(&bytes[offset..offset + count]);
                    if let Some(hasher) = hasher.as_mut() {
                        hasher.update(&out[written..written + count]);
                    }
                    *position += count as u64;
                    written += count;
                }
                if *position == *length {
                    verify_root(store, *root, *length, *digest, hasher, validated)?;
                }
                Ok(written)
            }
        }
    }
}

fn verify_root(
    store: &Store,
    root: u64,
    length: u64,
    digest: Digest,
    hasher: &mut Option<Hasher>,
    validated: &mut bool,
) -> io::Result<()> {
    if *validated {
        return Ok(());
    }
    let actual = if let Some(hasher) = hasher.take() {
        hasher.finish()
    } else {
        // A caller may seek into a file and read only its tail. Verify the
        // complete root once before reporting EOF, still one chunk at a time.
        let mut hasher = Hasher::new(length);
        let mut position = 0;
        while position < length {
            let chunk = locate_leaf(store, root, position).map_err(io_error)?;
            let bytes = store.bytes(chunk.id).map_err(io_error)?;
            if bytes.len() as u64 != chunk.length {
                return Err(io_error("Checkpoint chunk length changed".into()));
            }
            hasher.update(&bytes);
            position += chunk.length;
        }
        hasher.finish()
    };
    if actual != digest {
        return Err(io_error("Checkpoint file digest mismatch".into()));
    }
    *validated = true;
    Ok(())
}

impl Seek for ObjectReader<'_> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        match &mut self.inner {
            ReaderInner::Small(cursor) => cursor.seek(from),
            ReaderInner::Chunks {
                length,
                position,
                hasher,
                ..
            } => {
                let base = match from {
                    SeekFrom::Start(target) => i128::from(target),
                    SeekFrom::Current(delta) => i128::from(*position) + i128::from(delta),
                    SeekFrom::End(delta) => i128::from(*length) + i128::from(delta),
                };
                let target = u64::try_from(base)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid seek"))?;
                if target != *position {
                    *hasher = (target == 0).then(|| Hasher::new(*length));
                    *position = target;
                }
                Ok(target)
            }
        }
    }
}
