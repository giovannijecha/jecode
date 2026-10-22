//! Retention and compaction. When the archive outgrows its budget, history
//! older than a horizon is released by copying what stays into a new
//! generation. Targeted recovery keeps live before-images and metadata for
//! current files; pins and pending operations keep their required objects.
//! Processes still reading the old generation keep it until they move on.
use super::frame::{HEADER, ObjectMeta};
use super::identity::Location;
use super::index::Index;
use super::journal::{self, Entry, Journal};
use super::layout;
use super::locks::Exclusive;
use super::record::{Point, Record, State, Version};
use super::store::{Store, Transaction};
use std::collections::BTreeSet;
use std::path::PathBuf;

mod recovery;

/// Start while a second generation still has room beside the first one.
const TRIGGER_PERCENT: u64 = 40;
const TARGET_PERCENT: u64 = 25;
const BATCH_BYTES: usize = 8 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collection {
    /// Legacy budget horizon; targeted recovery has independent deadlines.
    pub horizon: u64,
    pub before: u64,
    pub after: u64,
    pub generation: u64,
    /// Required recovery states, pins or pending writes exceed the target.
    pub over_budget: bool,
}

/// Time-based retention or accepted-group collapse needs a generation move
/// even when the archive is well below its byte budget.
pub fn due(index: &Index, now: u64) -> bool {
    recovery::due(index, now)
}

/// Compacts the archive when it exceeds its budget, or always with `force`.
/// Returns `None` when nothing is due or another process is collecting.
pub fn collect(location: &Location, force: bool) -> Result<Option<Collection>, String> {
    let directory = &location.directory;
    let lock = Exclusive::try_acquire(&layout::gc_lock(directory))
        .map_err(|error| format!("Could not lock checkpoint retention: {error}"))?;
    let Some(_collecting) = lock else {
        return Ok(None);
    };
    let mut store = Store::open(location.clone())?;
    let before = store.journal.stored_bytes();
    let budget = store.policy.budget_bytes;
    let expired = due(&store.index, crate::checkpoints::clock::now());
    if !force && !expired && before <= budget / 100 * TRIGGER_PERCENT {
        return Ok(None);
    }
    let target = budget / 100 * TARGET_PERCENT;
    let horizon = choose(&store.index, target);
    let records = snapshot(&store.index, horizon);
    let (objects, projected) = retained_records(&store.index, &records);
    if !force && !expired && projected >= before {
        return Ok(None);
    }
    // Compaction briefly needs a second generation beside the active one.
    store.journal.ensure_scratch(projected)?;
    let old = store.journal.generation;
    let generation = old + 1;
    // A collection that crashed before finishing left an unused generation.
    let leftover = journal::generation_directory(directory, generation);
    {
        let _store = Exclusive::acquire(&layout::store_lock(directory))
            .map_err(|error| format!("Could not lock checkpoint archive: {error}"))?;
        if leftover.exists() {
            if let Ok(Some(_unused)) =
                Exclusive::try_acquire(&journal::generation_lock(directory, generation))
            {
                std::fs::remove_dir_all(&leftover).map_err(|error| {
                    format!("Could not remove an unfinished compaction: {error}")
                })?;
            } else {
                return Ok(None);
            }
        }
    }
    let mut output = Output {
        journal: Journal::open_generation(directory, generation)?,
        lock_path: layout::store_lock(directory),
        under_store_lock: false,
        buffer: Vec::new(),
        bytes: 0,
    };
    output.journal.min_free_bytes = store.policy.min_free_bytes;
    #[cfg(test)]
    let mut first = true;
    let copy = (|| {
        for id in &objects {
            output.object(&store, *id)?;
            #[cfg(test)]
            if std::mem::take(&mut first) {
                super::tests::process::stage("gc-first-object-copied");
            }
        }
        for record in &records {
            output.record(record)?;
        }
        Ok::<(), String>(())
    })();
    if let Err(error) = copy {
        drop(output);
        let _store = Exclusive::acquire(&layout::store_lock(directory))
            .map_err(|lock| format!("{error}; could not lock unfinished compaction: {lock}"))?;
        std::fs::remove_dir_all(&leftover).map_err(|cleanup| {
            format!("{error}; could not remove unfinished compaction: {cleanup}")
        })?;
        return Err(error);
    }
    let copied = Copied {
        objects,
        latest: store.index.latest(),
        pins: store.index.pins.keys().copied().collect(),
    };
    let mut input = Journal::open_generation(directory, old)?;
    input.seek(store.journal.position());
    let after = store.transaction(true, |transaction| {
        output.under_store_lock = true;
        tail(transaction, &mut input, &mut output, &copied)
    })?;
    #[cfg(test)]
    super::tests::process::stage("gc-moved");
    {
        let _lock = Exclusive::acquire(&layout::store_lock(directory))
            .map_err(|error| format!("Could not lock checkpoint archive: {error}"))?;
        let next = generation.to_string();
        super::quota::admit(directory, next.len() as u64, true, u64::MAX)?;
        super::files::atomic_write(&layout::current(directory), next.as_bytes())
            .map_err(|error| format!("Could not update checkpoint archive: {error}"))?;
        super::files::sync_directory(&layout::history(directory))
            .map_err(|error| format!("Could not flush checkpoint archive directory: {error}"))?;
    }
    drop(input);
    drop(store);
    remove_older(directory, generation);
    Ok(Some(Collection {
        horizon,
        before,
        after,
        generation,
        over_budget: after > target,
    }))
}

/// What the snapshot copied, for checking the appends made meanwhile.
struct Copied {
    objects: BTreeSet<u64>,
    latest: u64,
    pins: BTreeSet<u64>,
}

/// Copies appends made during the snapshot copy, then moves every process.
fn tail(
    transaction: &mut Transaction<'_>,
    input: &mut Journal,
    output: &mut Output,
    copied: &Copied,
) -> Result<u64, String> {
    let mut entries = Vec::new();
    input.catch_up(&mut |entry| entries.push(entry))?;
    let index = transaction.index();
    let mut missing = BTreeSet::new();
    // Appends may refer to released objects, such as a restore of old history.
    let refer = |state: &State, missing: &mut BTreeSet<u64>| {
        if let State::File { object, .. } = state
            && *object <= copied.latest
        {
            bases(index, *object, missing);
        }
    };
    for entry in &entries {
        match entry {
            Entry::Record(Record::Pin {
                target,
                pinned: true,
                ..
            }) if !copied.pins.contains(target) => {
                return Err("a new pin was added during compaction; retry later".into());
            }
            Entry::Record(Record::Version(version)) => refer(&version.state, &mut missing),
            Entry::Record(Record::Prepare(prepare)) => {
                refer(&prepare.before, &mut missing);
                refer(&prepare.after, &mut missing);
            }
            Entry::Record(Record::Recovery(super::record::Recovery::Step { before, .. })) => {
                refer(before, &mut missing)
            }
            Entry::Object(meta, _, chunks) => {
                if let super::frame::Encoding::Delta { base, .. } = meta.encoding
                    && base <= copied.latest
                {
                    bases(index, base, &mut missing);
                }
                for chunk in chunks {
                    if *chunk <= copied.latest {
                        bases(index, *chunk, &mut missing);
                    }
                }
            }
            _ => {}
        }
    }
    for id in missing.difference(&copied.objects) {
        output.object(transaction.store, *id)?;
    }
    for entry in entries {
        match entry {
            Entry::Record(record) => output.record(&record)?,
            Entry::Object(meta, location, _) => {
                let payload = transaction.store.journal.read(&location)?;
                output.raw(&meta.encode(&payload[ObjectMeta::SIZE.min(payload.len())..]))?;
            }
        }
    }
    let moved = transaction.seq();
    // Later sequence numbers continue after every number already used.
    output.record(&marker("watermark", moved))?;
    output.finish()?;
    transaction.refresh_admission();
    // The old journal must never publish Moved before the new directory entry
    // is stable; recovery follows that marker even if CURRENT is still old.
    super::files::sync_directory(&layout::history(&transaction.store.workspace.directory))
        .map_err(|error| format!("Could not flush checkpoint archive directory: {error}"))?;
    transaction.record(Record::Moved {
        seq: moved,
        generation: output.journal.generation,
    });
    Ok(output.bytes)
}

struct Output {
    journal: Journal,
    lock_path: PathBuf,
    under_store_lock: bool,
    buffer: Vec<u8>,
    bytes: u64,
}

impl Output {
    fn object(&mut self, store: &Store, id: u64) -> Result<(), String> {
        let object = store
            .index
            .objects
            .get(&id)
            .ok_or_else(|| format!("Checkpoint object {id} is not available"))?;
        let payload = store.journal.read(&object.location)?;
        self.raw(
            &object
                .meta
                .encode(&payload[ObjectMeta::SIZE.min(payload.len())..]),
        )
    }

    fn record(&mut self, record: &Record) -> Result<(), String> {
        self.raw(&record.frame())
    }

    fn raw(&mut self, frame: &[u8]) -> Result<(), String> {
        self.bytes = self.bytes.saturating_add(frame.len() as u64);
        self.buffer.extend_from_slice(frame);
        if self.buffer.len() >= BATCH_BYTES {
            self.flush_buffer()?;
        }
        Ok(())
    }

    fn flush_buffer(&mut self) -> Result<(), String> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        // Snapshot copying runs alongside writers. Count and append under the
        // same store lock so neither side can spend the other's headroom.
        let _lock = if self.under_store_lock {
            None
        } else {
            Some(Exclusive::acquire(&self.lock_path).map_err(|error| {
                format!("Could not lock checkpoint archive during compaction: {error}")
            })?)
        };
        self.journal.append(&self.buffer, false)?;
        self.buffer.clear();
        Ok(())
    }

    fn finish(&mut self) -> Result<(), String> {
        self.flush_buffer()?;
        self.journal.sync()
    }
}

fn marker(kind: &str, seq: u64) -> Record {
    Record::Point(Point {
        seq,
        kind: kind.into(),
        label: String::new(),
        origin: "retention".into(),
        time: crate::checkpoints::clock::now(),
        policy: 0,
        pinned: false,
    })
}

/// The smallest horizon whose retained state fits the target. A long turn can
/// have many versions after its last named point, so those are candidates too.
fn choose(index: &Index, target: u64) -> u64 {
    let earliest = index
        .points
        .keys()
        .next()
        .copied()
        .unwrap_or(index.horizon)
        .max(index.horizon);
    let mut candidates: Vec<u64> = index
        .points
        .keys()
        .copied()
        .filter(|seq| *seq >= earliest)
        .collect();
    candidates.extend(
        index
            .paths
            .values()
            .flat_map(|versions| versions.iter().map(|version| version.seq))
            .filter(|seq| *seq >= earliest),
    );
    candidates.push(index.latest().max(index.horizon));
    candidates.sort_unstable();
    candidates.dedup();
    let fits = |horizon: u64| retained(index, horizon).1 <= target;
    let count = candidates.partition_point(|horizon| !fits(*horizon));
    candidates
        .get(count)
        .copied()
        .unwrap_or(index.latest())
        .max(index.horizon)
}

/// Versions that stay with `horizon`: each path's state at the horizon and
/// at every pin, and everything later.
fn kept_versions(index: &Index, horizon: u64) -> Vec<&Version> {
    let mut anchors: Vec<u64> = index
        .pins
        .keys()
        .copied()
        .filter(|pin| *pin < horizon)
        .collect();
    if let Some(latest_point) = index.points.keys().next_back().copied()
        && latest_point < horizon
    {
        anchors.push(latest_point);
    }
    let complete = index.coverage_at(horizon);
    let mut kept = Vec::new();
    for versions in index.paths.values() {
        let visible = versions.partition_point(|version| version.seq <= horizon);
        let start = visible.saturating_sub(1);
        let only_absence = visible == versions.len()
            && versions
                .last()
                .is_some_and(|version| !version.state.exists());
        let mut chosen: BTreeSet<usize> = BTreeSet::new();
        // With complete coverage, an unrecorded path is known to be absent.
        if !(only_absence && complete) {
            chosen.extend(start..versions.len());
        }
        for anchor in &anchors {
            let count = versions.partition_point(|version| version.seq <= *anchor);
            if let Some(index) = count.checked_sub(1) {
                chosen.insert(index);
            }
        }
        kept.extend(chosen.into_iter().map(|index| &versions[index]));
    }
    kept
}

/// Objects needed by the kept versions and pending operations, with their
/// delta bases, and their stored size.
fn retained(index: &Index, horizon: u64) -> (BTreeSet<u64>, u64) {
    let records = snapshot(index, horizon);
    retained_records(index, &records)
}

fn retained_records(index: &Index, records: &[Record]) -> (BTreeSet<u64>, u64) {
    let mut objects = BTreeSet::new();
    let add = |state: &State, objects: &mut BTreeSet<u64>| {
        if let State::File { object, .. } = state {
            bases(index, *object, objects);
        }
    };
    for record in records {
        match record {
            Record::Version(version) => add(&version.state, &mut objects),
            Record::Prepare(prepare) => {
                add(&prepare.before, &mut objects);
                add(&prepare.after, &mut objects);
            }
            Record::Recovery(super::record::Recovery::Step { before, .. }) => {
                add(before, &mut objects);
            }
            _ => {}
        }
    }
    let object_bytes: u64 = objects
        .iter()
        .filter_map(|id| index.objects.get(id))
        .map(|object| object.location.length + HEADER as u64)
        .sum();
    let record_bytes: u64 = records
        .iter()
        .map(|record| record.frame().len() as u64)
        .sum();
    let watermark = marker("watermark", index.latest()).frame().len() as u64;
    (
        objects,
        object_bytes
            .saturating_add(record_bytes)
            .saturating_add(watermark),
    )
}

fn bases(index: &Index, id: u64, objects: &mut BTreeSet<u64>) {
    let mut pending = vec![id];
    while let Some(id) = pending.pop() {
        if !objects.insert(id) {
            continue;
        }
        match index.objects.get(&id).map(|object| object.meta.encoding) {
            Some(super::frame::Encoding::Delta { base, .. }) => pending.push(base),
            Some(super::frame::Encoding::Chunks { .. }) => {
                if let Some(chunks) = index.manifests.get(&id) {
                    pending.extend(chunks.iter().copied());
                }
            }
            _ => {}
        }
    }
}

/// Records of the new generation, in sequence order.
fn snapshot(index: &Index, horizon: u64) -> Vec<Record> {
    if index.recovery.mode.is_some() {
        return recovery::snapshot(index, horizon, crate::checkpoints::clock::now());
    }
    let mut anchors: Vec<u64> = index.pins.keys().copied().collect();
    let latest_point = index.points.keys().next_back().copied();
    if let Some(latest_point) = latest_point {
        anchors.push(latest_point);
    }
    let mut records: Vec<Record> = Vec::new();
    let mut last: std::collections::HashMap<&str, &Version> = std::collections::HashMap::new();
    for version in kept_versions(index, horizon) {
        records.push(Record::Version(version.clone()));
        let newest = last.entry(version.path.as_str()).or_insert(version);
        if version.seq > newest.seq {
            *newest = version;
        }
    }
    // Metadata observed after the newest version, without a content change.
    for (path, version) in last {
        if let Some(stat) = index.stats.get(path)
            && version.stat.as_ref() != Some(stat)
        {
            records.push(Record::Stat {
                seq: version.seq,
                path: path.to_string(),
                stat: *stat,
            });
        }
    }
    records.extend(index.pending.values().cloned().map(Record::Prepare));
    records.extend(
        index
            .outcomes
            .range(horizon..)
            .map(|(operation, outcome)| Record::Complete {
                seq: *operation,
                operation: *operation,
                outcome: *outcome,
            }),
    );
    records.extend(
        index
            .points
            .values()
            .filter(|point| {
                point.seq >= horizon
                    || index.pins.contains_key(&point.seq)
                    || latest_point == Some(point.seq)
            })
            .cloned()
            .map(Record::Point),
    );
    records.extend(index.pins.iter().map(|(target, label)| Record::Pin {
        seq: *target,
        target: *target,
        pinned: true,
        label: label.clone(),
    }));
    let needed = |seq: u64, starts: &[u64]| seq >= horizon || starts.contains(&seq);
    let policy_starts: Vec<u64> = anchors
        .iter()
        .chain(std::iter::once(&horizon))
        .filter_map(|at| {
            index
                .policies
                .range(..=*at)
                .next_back()
                .map(|(seq, _)| *seq)
        })
        .collect();
    records.extend(
        index
            .policies
            .iter()
            .filter(|(seq, _)| needed(**seq, &policy_starts))
            .map(|(seq, (id, policy))| Record::Policy {
                seq: *seq,
                id: *id,
                text: policy.encode(),
            }),
    );
    let coverage_starts: Vec<u64> = anchors
        .iter()
        .chain(std::iter::once(&horizon))
        .filter_map(|at| {
            index
                .coverage
                .range(..=*at)
                .next_back()
                .map(|(seq, _)| *seq)
        })
        .collect();
    records.extend(
        index
            .coverage
            .iter()
            .filter(|(seq, _)| needed(**seq, &coverage_starts))
            .map(|(seq, (complete, reason))| Record::Coverage {
                seq: *seq,
                complete: *complete,
                reason: reason.clone(),
            }),
    );
    records.extend(index.events.iter().cloned().map(Record::Event));
    records.push(marker("horizon", horizon));
    records.sort_by_key(Record::seq);
    records
}

/// Deletes generations older than `current` that no process still reads.
fn remove_older(directory: &std::path::Path, current: u64) {
    let Ok(_store) = Exclusive::acquire(&layout::store_lock(directory)) else {
        return;
    };
    for generation in 1..current {
        let path = journal::generation_directory(directory, generation);
        if !path.exists() {
            continue;
        }
        if let Ok(Some(_held)) =
            Exclusive::try_acquire(&journal::generation_lock(directory, generation))
        {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

#[cfg(test)]
#[path = "gc/quota_tests.rs"]
mod quota_tests;
