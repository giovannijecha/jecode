//! One process's handle on a shared workspace archive: replayed index, short
//! store-lock transactions and verified object reconstruction.
mod cache;
use super::frame::{Encoding, HEADER, ObjectMeta};
use super::hash::{self, Digest};
use super::identity::Location as Workspace;
use super::index::{Index, MAX_DEPTH};
use super::journal::{self, Journal, Location};
use super::locks::{Exclusive, Owner};
use super::policy::Policy;
use super::record::{Event, Point, Record};
use cache::Cache;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct Store {
    pub workspace: Workspace,
    pub policy: Policy,
    pub journal: Journal,
    pub index: Index,
    pub owner: String,
    _owner: Owner,
    cache: Mutex<Cache>,
    /// Counters for measurements and status.
    pub counters: Counters,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Counters {
    pub transactions: u64,
    pub flushes: u64,
    pub frames: u64,
    pub bytes_written: u64,
    pub objects_full: u64,
    pub objects_delta: u64,
    pub objects_reused: u64,
    pub files_read: u64,
    pub bytes_read: u64,
    pub scans: u64,
}

impl Store {
    pub fn lock_path(&self) -> PathBuf {
        self.workspace.directory.join("store.lock")
    }

    pub fn open(workspace: Workspace) -> Result<Self, String> {
        let directory = &workspace.directory;
        let owner = super::files::unique("owner");
        let owner_lock = Owner::acquire(directory.join("owners").join(format!("{owner}.lock")))
            .map_err(|error| format!("Could not register with checkpoint archive: {error}"))?;
        let _lock = Exclusive::acquire(&directory.join("store.lock"))
            .map_err(|error| format!("Could not lock checkpoint archive: {error}"))?;
        let edited = Policy::read(directory)?;
        let journal = Journal::open(directory)?;
        let mut store = Self {
            workspace,
            policy: edited.clone().unwrap_or_default(),
            journal,
            index: Index::default(),
            owner,
            _owner: owner_lock,
            cache: Mutex::new(Cache::default()),
            counters: Counters::default(),
        };
        store.catch_up()?;
        store.remove_stale_generations();
        store.adopt(edited)?;
        Ok(store)
    }

    /// Applies a hand-edited `policy.json`; the journal tells other processes.
    /// A missing file is written so users can find and edit it.
    fn adopt(&mut self, edited: Option<Policy>) -> Result<(), String> {
        let recorded = self.index.policies.values().next_back().cloned();
        let previous = recorded
            .as_ref()
            .map(|(_, policy)| policy.clone())
            .unwrap_or_default();
        let policy = edited.unwrap_or_else(|| {
            let _ = self.policy.save_locked(&self.workspace.directory);
            self.policy.clone()
        });
        if recorded.as_ref().map(|(id, _)| *id) == Some(policy.id()) {
            return Ok(());
        }
        let mut transaction = Transaction::new(self);
        let seq = transaction.seq();
        let (id, text) = (policy.id(), policy.encode());
        transaction.record(Record::Policy { seq, id, text });
        if let Some((_, previous)) = recorded {
            let seq = transaction.seq();
            transaction.record(Record::Event(Event {
                seq,
                origin: "policy".into(),
                time: crate::sessions::now(),
                text: format!(
                    "checkpoint scope changed in policy.json: {}",
                    policy.changes(&previous).join(", ")
                ),
                paths: Vec::new(),
            }));
        }
        transaction.store.policy = policy;
        match transaction.commit(false) {
            Err(error) if super::quota::is_full(&error) => {
                // A legacy archive can already exceed the hard cap. Keep it
                // readable and leave the policy from its last valid record.
                self.policy = previous;
                Ok(())
            }
            result => result,
        }
    }

    /// Replays other processes' appends; follows a compaction to its new generation.
    fn catch_up(&mut self) -> Result<(), String> {
        let index = &mut self.index;
        self.journal.catch_up(&mut |entry| index.apply(entry))?;
        // Compaction writes Moved before CURRENT; a crash between them is
        // repaired by the first process that follows the move.
        while let Some(generation) = self.index.moved {
            let directory = &self.workspace.directory;
            let journal = Journal::open_existing_generation(directory, generation)?;
            self.journal = journal;
            self.index = Index::default();
            self.cache = Mutex::new(Cache::default());
            let index = &mut self.index;
            self.journal.catch_up(&mut |entry| index.apply(entry))?;
            self.remove_stale_generations();
        }
        self.repair_current()?;
        // Limits changed by another process apply here too.
        if let Some((_, (id, policy))) = self.index.policies.iter().next_back()
            && *id != self.policy.id()
        {
            self.policy = policy.clone();
        }
        Ok(())
    }

    /// A full archive may delay the small CURRENT repair after Moved. Keep
    /// following the published generation, and retry on later transactions.
    fn repair_current(&self) -> Result<(), String> {
        let directory = &self.workspace.directory;
        let generation = self.journal.generation;
        if journal::current_generation(directory)? >= generation {
            return Ok(());
        }
        let next = generation.to_string();
        match super::quota::admit(directory, next.len() as u64, true, super::quota::LIMIT) {
            Ok(()) => super::files::atomic_write(&directory.join("CURRENT"), next.as_bytes())
                .map_err(|error| format!("Could not update checkpoint archive: {error}")),
            Err(error) if super::quota::is_full(&error) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Runs `body` under the store lock with an up-to-date index.
    pub fn transaction<T>(
        &mut self,
        durable: bool,
        body: impl FnOnce(&mut Transaction<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        let _lock = Exclusive::acquire(&self.lock_path())
            .map_err(|error| format!("Could not lock checkpoint archive: {error}"))?;
        self.catch_up()?;
        let mut transaction = Transaction::new(self);
        let value = body(&mut transaction)?;
        transaction.commit(durable)?;
        Ok(value)
    }

    /// Refreshes the index from the shared journal.
    pub fn refresh(&mut self) -> Result<(), String> {
        let _lock = Exclusive::acquire(&self.lock_path())
            .map_err(|error| format!("Could not lock checkpoint archive: {error}"))?;
        self.catch_up()
    }

    /// Follows peer compaction and releases old generations after readers leave.
    pub fn maintain(&mut self) -> Result<(), String> {
        let _lock = Exclusive::acquire(&self.lock_path())
            .map_err(|error| format!("Could not lock checkpoint archive: {error}"))?;
        self.catch_up()?;
        self.remove_stale_generations();
        Ok(())
    }

    /// Exact bytes of an object, verified against its recorded digest.
    pub fn bytes(&self, id: u64) -> Result<Arc<[u8]>, String> {
        if let Some(bytes) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).get(id) {
            return Ok(bytes);
        }
        let object = *self
            .index
            .objects
            .get(&id)
            .ok_or_else(|| format!("Checkpoint object {id} is not available"))?;
        let payload = self.journal.read(&object.location)?;
        let data = &payload[ObjectMeta::SIZE.min(payload.len())..];
        let bytes: Arc<[u8]> = match object.meta.encoding {
            Encoding::Full => data.into(),
            Encoding::Delta {
                base,
                prefix,
                suffix,
            } => {
                let base = self.bytes(base)?;
                let (prefix, suffix) = (prefix as usize, suffix as usize);
                if prefix + suffix > base.len() {
                    return Err(format!("Checkpoint object {id} has an invalid delta"));
                }
                let mut bytes = Vec::with_capacity(object.meta.length as usize);
                bytes.extend_from_slice(&base[..prefix]);
                bytes.extend_from_slice(data);
                bytes.extend_from_slice(&base[base.len() - suffix..]);
                bytes.into()
            }
        };
        if bytes.len() as u64 != object.meta.length || hash::digest(&bytes) != object.meta.digest {
            return Err(format!(
                "Checkpoint object {id} failed verification; its bytes are damaged"
            ));
        }
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, bytes.clone());
        Ok(bytes)
    }

    fn remember(&self, id: u64, bytes: Arc<[u8]>) {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, bytes);
    }

    /// Deletes generations that no process still reads.
    pub fn remove_stale_generations(&self) {
        let directory = &self.workspace.directory;
        let Ok(current) = journal::current_generation(directory) else {
            return;
        };
        let oldest_needed = current.min(self.journal.generation);
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(generation) = name
                .strip_prefix("gen-")
                .filter(|rest| !rest.contains('.'))
                .and_then(|rest| rest.parse::<u64>().ok())
            else {
                continue;
            };
            // Newer generations may be a compaction in progress.
            if generation >= oldest_needed {
                continue;
            }
            let lock = journal::generation_lock(directory, generation);
            if let Ok(Some(_held)) = Exclusive::try_acquire(&lock) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

/// Objects written by an unflushed transaction, keyed by content.
type Fresh = HashMap<(Digest, u64), Vec<(u64, Arc<[u8]>)>>;

/// Frames buffered under the store lock and appended in one write.
pub struct Transaction<'a> {
    pub store: &'a mut Store,
    buffer: Vec<u8>,
    objects: Vec<(ObjectMeta, usize, u64, u64)>,
    records: Vec<Record>,
    fresh: Fresh,
    used: Option<super::quota::Usage>,
}

impl<'a> Transaction<'a> {
    fn new(store: &'a mut Store) -> Self {
        Self {
            store,
            buffer: Vec::new(),
            objects: Vec::new(),
            records: Vec::new(),
            fresh: HashMap::new(),
            used: None,
        }
    }

    pub fn seq(&mut self) -> u64 {
        let seq = self.store.index.next;
        self.store.index.next += 1;
        seq
    }

    pub fn index(&self) -> &Index {
        &self.store.index
    }

    /// Another journal wrote under this same store lock (compaction tail).
    pub fn refresh_admission(&mut self) {
        self.used = None;
    }

    pub fn record(&mut self, record: Record) {
        self.buffer.extend_from_slice(&record.frame());
        self.records.push(record);
    }

    pub fn point(&mut self, kind: &str, label: &str, origin: &str, pinned: bool) -> u64 {
        let seq = self.seq();
        let policy = self.store.policy.id();
        self.record(Record::Point(Point {
            seq,
            kind: kind.into(),
            label: label.into(),
            origin: origin.into(),
            time: crate::sessions::now(),
            policy,
            pinned,
        }));
        seq
    }

    /// Stores bytes once, as a delta of `base` when that saves at least half.
    /// Equal digests only nominate reuse; the bytes are compared first.
    pub fn object(&mut self, bytes: Arc<[u8]>, base: Option<u64>) -> Result<u64, String> {
        if bytes.len() as u64 > super::policy::DEFAULT_MAX_FILE_BYTES {
            return Err("Checkpoint file exceeds the hard 10 MiB capture limit".into());
        }
        let digest = hash::digest(&bytes);
        let key = (digest, bytes.len() as u64);
        if let Some(candidates) = self.store.index.contents.get(&key) {
            for id in candidates.clone() {
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
        ));
        self.buffer.extend_from_slice(&frame);
        self.fresh.entry(key).or_default().push((id, bytes.clone()));
        self.store.remember(id, bytes);
        Ok(id)
    }

    fn delta(&self, base: u64, bytes: &[u8]) -> Option<(Encoding, std::ops::Range<usize>)> {
        let depth = self.store.index.objects.get(&base)?.depth;
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

    /// Appends buffered frames and applies them to the index. Sequence numbers
    /// were reserved in the index while building the transaction.
    pub fn flush(&mut self, durable: bool) -> Result<(), String> {
        if self.buffer.is_empty() {
            if durable {
                self.store.journal.sync()?;
            }
            return Ok(());
        }
        let terminal = self.objects.is_empty()
            && !self.records.is_empty()
            && self.records.iter().all(|record| {
                matches!(
                    record,
                    Record::Complete { .. } | Record::Coverage { .. } | Record::Moved { .. }
                )
            });
        let used = match self.used {
            Some(used) => used,
            None => super::quota::measure(
                &self.store.workspace.directory,
                self.store.journal.generation,
            )?,
        };
        let result = self.store.journal.append_counted(
            &self.buffer,
            durable,
            terminal,
            self.objects.is_empty(),
            used,
        );
        let next = self.store.index.next;
        let (segment, start) = match result {
            Ok(position) => position,
            Err(error) => {
                self.used = None;
                // Forget reserved numbers and cached bytes of unwritten objects.
                self.buffer.clear();
                self.records.clear();
                for (meta, ..) in self.objects.drain(..) {
                    self.store
                        .cache
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(meta.id);
                }
                self.fresh.clear();
                return Err(error);
            }
        };
        let counters = &mut self.store.counters;
        let added = self.buffer.len() as u64;
        self.used = Some(super::quota::Usage {
            total: used.total.saturating_add(added),
            generation: used.generation.saturating_add(added),
        });
        counters.flushes += u64::from(durable);
        counters.bytes_written += self.buffer.len() as u64;
        counters.frames += (self.objects.len() + self.records.len()) as u64;
        for (meta, offset, length, checksum) in self.objects.drain(..) {
            let location = Location {
                segment,
                offset: start + offset as u64,
                length,
                checksum,
            };
            self.store.index.object(meta, location);
        }
        for record in self.records.drain(..) {
            self.store.index.record(record);
        }
        self.store.index.next = self.store.index.next.max(next);
        self.buffer.clear();
        self.fresh.clear();
        Ok(())
    }

    fn commit(mut self, durable: bool) -> Result<(), String> {
        self.store.counters.transactions += 1;
        self.flush(durable)
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        // A body can fail after buffering objects but before its first flush.
        // Their cache entries must not outlive the unwritten frames.
        let mut cache = self.store.cache.lock().unwrap_or_else(|e| e.into_inner());
        for (meta, ..) in &self.objects {
            cache.remove(meta.id);
        }
    }
}
