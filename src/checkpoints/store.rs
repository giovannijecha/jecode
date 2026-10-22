//! One process's handle on a shared workspace archive: replayed index, short
//! store-lock transactions and verified object reconstruction.
mod cache;
mod objects;
mod reader;
mod stream;
use super::frame::{Encoding, ObjectMeta};
use super::hash::{self, Digest};
use super::identity::Location as Workspace;
use super::index::Index;
use super::journal::{self, Journal, Location};
use super::locks::{Exclusive, Owner};
use super::policy::Policy;
use super::record::{Event, Point, Record};
use super::{layout, lifecycle};
use cache::Cache;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub const CHUNK_BYTES: usize = 1 << 20;

pub struct Store {
    pub workspace: Workspace,
    pub policy: Policy,
    pub journal: Journal,
    pub index: Index,
    pub owner: String,
    history_floor: u64,
    _lifecycle: lifecycle::Lease,
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
        layout::store_lock(&self.workspace.directory)
    }

    pub fn open(workspace: Workspace) -> Result<Self, String> {
        let directory = workspace.directory.clone();
        if layout::version(&directory) == Some(2) {
            layout::guard_legacy(&directory)?;
        }
        let lifecycle = lifecycle::open(&workspace)?;
        let owner = super::files::unique("owner");
        let owner_lock = Owner::acquire(layout::owner_lock(&directory, &owner))
            .map_err(|error| format!("Could not register with checkpoint archive: {error}"))?;
        let _lock = Exclusive::acquire(&layout::store_lock(&directory))
            .map_err(|error| format!("Could not lock checkpoint archive: {error}"))?;
        let edited = Policy::read_with_migration(&directory)?;
        let journal = Journal::open(&directory)?;
        let history_floor = lifecycle::high_water(&directory)?;
        let mut store = Self {
            workspace,
            policy: edited
                .as_ref()
                .map(|(policy, _)| policy.clone())
                .unwrap_or_default(),
            journal,
            index: Index::default(),
            owner,
            history_floor,
            _lifecycle: lifecycle,
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
    fn adopt(&mut self, edited: Option<(Policy, bool)>) -> Result<(), String> {
        let recorded = self.index.policies.values().next_back().cloned();
        let previous = recorded
            .as_ref()
            .map(|(_, policy)| policy.clone())
            .unwrap_or_default();
        let rewrite = edited.as_ref().is_some_and(|(_, migrated)| *migrated);
        let policy = edited.map(|(policy, _)| policy).unwrap_or_else(|| {
            let _ = self.policy.save_locked(&self.workspace.directory);
            self.policy.clone()
        });
        if recorded.as_ref().map(|(id, _)| *id) == Some(policy.id()) {
            if rewrite {
                policy.save_locked(&self.workspace.directory)?;
            }
            return Ok(());
        }
        let mut transaction = Transaction::new(self);
        let seq = transaction.seq();
        let (id, text) = (policy.id(), policy.encode());
        transaction.record(Record::Policy { seq, id, text });
        if let Some((_, previous)) = recorded {
            let changes = policy.changes(&previous);
            if !changes.is_empty() {
                let seq = transaction.seq();
                transaction.record(Record::Event(Event {
                    seq,
                    origin: "policy".into(),
                    time: crate::sessions::now(),
                    text: format!(
                        "checkpoint scope changed in policy.json: {}",
                        changes.join(", ")
                    ),
                    paths: Vec::new(),
                }));
            }
        }
        transaction.store.policy = policy;
        match transaction.commit(false) {
            Err(error) if super::quota::is_full(&error) => {
                // A legacy archive can already exceed the hard cap. Keep it
                // readable and leave the policy from its last valid record.
                self.policy = previous;
                Ok(())
            }
            Ok(()) => {
                if rewrite {
                    self.policy.save_locked(&self.workspace.directory)?;
                }
                Ok(())
            }
            Err(error) => Err(error),
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
        // Expiry preserves sequence identity: old cp-N references must stay
        // below the retained horizon even after new history or compaction.
        self.index.next = self.index.next.max(self.history_floor);
        self.index.horizon = self.index.horizon.max(self.history_floor);
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
        match super::quota::admit(directory, next.len() as u64, true, u64::MAX) {
            Ok(()) => super::files::atomic_write(&layout::current(directory), next.as_bytes())
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
            Encoding::Chunks { .. } => {
                let mut reader = self.reader(id)?;
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut reader, &mut bytes)
                    .map_err(|error| format!("Could not read checkpoint file: {error}"))?;
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
        let Ok(entries) = std::fs::read_dir(layout::history(directory)) else {
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
    objects: Vec<(ObjectMeta, usize, u64, u64, Vec<u64>)>,
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
        #[cfg(test)]
        let used = if self.store.journal.quota_limit == u64::MAX {
            super::quota::Usage {
                total: 0,
                generation: 0,
            }
        } else {
            match self.used {
                Some(used) => used,
                None => super::quota::measure(
                    &self.store.workspace.directory,
                    self.store.journal.generation,
                )?,
            }
        };
        #[cfg(not(test))]
        let used = super::quota::Usage {
            total: 0,
            generation: 0,
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
        for (meta, offset, length, checksum, chunks) in self.objects.drain(..) {
            let location = Location {
                segment,
                offset: start + offset as u64,
                length,
                checksum,
            };
            self.store.index.object(meta, location, chunks);
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
