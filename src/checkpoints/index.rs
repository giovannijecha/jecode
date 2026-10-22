//! In-memory view of the archive rebuilt from the journal. Every process
//! derives the same facts from the shared journal; nothing here is private
//! state that other sessions could contradict.
use super::frame::{Encoding, ObjectMeta};
use super::hash::Digest;
use super::journal::{Entry, Location};
use super::policy::Policy;
use super::record::{Event, Outcome, Point, Prepare, Record, Stat, State, Version};
use std::collections::{BTreeMap, HashMap, VecDeque};

pub const MAX_DEPTH: u32 = 16;
pub(super) const EVENT_LIMIT: usize = 256;

#[derive(Clone, Copy, Debug)]
pub struct Object {
    pub meta: ObjectMeta,
    pub location: Location,
    pub depth: u32,
}

#[derive(Clone, Default)]
pub struct Index {
    pub next: u64,
    pub objects: HashMap<u64, Object>,
    /// Chunk references of manifest objects; every referenced chunk is an
    /// ordinary verified object in `objects`.
    pub manifests: HashMap<u64, Vec<u64>>,
    pub contents: HashMap<(Digest, u64), Vec<u64>>,
    pub paths: BTreeMap<String, Vec<Version>>,
    pub stats: HashMap<String, Stat>,
    pub points: BTreeMap<u64, Point>,
    pub pins: BTreeMap<u64, String>,
    /// Prepared operations without a completion record.
    pub pending: BTreeMap<u64, Prepare>,
    pub outcomes: BTreeMap<u64, Outcome>,
    pub events: VecDeque<Event>,
    pub policies: BTreeMap<u64, (u64, Policy)>,
    pub coverage: BTreeMap<u64, (bool, String)>,
    /// Sequences below the horizon were released by retention.
    pub horizon: u64,
    pub moved: Option<u64>,
    /// Sequence of the newest path version.
    pub last_change: u64,
    /// Bytes of object frames; the journal also contains metadata records.
    pub object_bytes: u64,
}

impl Index {
    pub fn apply(&mut self, entry: Entry) {
        match entry {
            Entry::Object(meta, location, chunks) => self.object(meta, location, chunks),
            Entry::Record(record) => self.record(record),
        }
    }

    pub fn object(&mut self, meta: ObjectMeta, location: Location, chunks: Vec<u64>) {
        self.next = self.next.max(meta.id + 1);
        let depth = match meta.encoding {
            Encoding::Full => 0,
            Encoding::Chunks { .. } => 0,
            Encoding::Delta { base, .. } => self
                .objects
                .get(&base)
                .map_or(MAX_DEPTH + 1, |base| base.depth + 1),
        };
        self.object_bytes += location.length + super::frame::HEADER as u64;
        self.contents
            .entry((meta.digest, meta.length))
            .or_default()
            .push(meta.id);
        if matches!(meta.encoding, Encoding::Chunks { .. }) {
            self.manifests.insert(meta.id, chunks);
        }
        self.objects.insert(
            meta.id,
            Object {
                meta,
                location,
                depth,
            },
        );
    }

    pub fn record(&mut self, record: Record) {
        self.next = self.next.max(record.seq() + 1);
        match record {
            Record::Version(version) => {
                self.last_change = self.last_change.max(version.seq);
                if let Some(stat) = version.stat {
                    self.stats.insert(version.path.clone(), stat);
                } else {
                    self.stats.remove(&version.path);
                }
                let versions = self.paths.entry(version.path.clone()).or_default();
                let at = versions.partition_point(|old| old.seq < version.seq);
                versions.insert(at, version);
            }
            Record::Stat { path, stat, .. } => {
                self.stats.insert(path, stat);
            }
            Record::Prepare(prepare) => {
                self.pending.insert(prepare.seq, prepare);
            }
            Record::Complete {
                operation, outcome, ..
            } => {
                self.pending.remove(&operation);
                self.outcomes.insert(operation, outcome);
            }
            Record::Point(point) => {
                if point.kind == "watermark" {
                    return;
                }
                if point.kind == "horizon" {
                    self.horizon = self.horizon.max(point.seq);
                    return;
                }
                if point.pinned {
                    self.pins.insert(point.seq, point.label.clone());
                }
                self.points.insert(point.seq, point);
            }
            Record::Pin {
                target,
                pinned,
                label,
                ..
            } => {
                if pinned {
                    self.pins.insert(target, label);
                } else {
                    self.pins.remove(&target);
                }
            }
            Record::Policy { seq, id, text } => {
                let policy = Policy::parse(&text).unwrap_or_default();
                self.policies.insert(seq, (id, policy));
            }
            Record::Event(event) => {
                self.events.push_back(event);
                while self.events.len() > EVENT_LIMIT {
                    self.events.pop_front();
                }
            }
            Record::Coverage {
                seq,
                complete,
                reason,
            } => {
                self.coverage.insert(seq, (complete, reason));
            }
            Record::Moved { generation, .. } => self.moved = Some(generation),
        }
    }

    pub fn latest(&self) -> u64 {
        self.next.saturating_sub(1)
    }

    pub fn current(&self, path: &str) -> Option<&Version> {
        self.paths.get(path).and_then(|versions| versions.last())
    }

    pub fn current_state(&self, path: &str) -> State {
        self.current(path)
            .map_or(State::Absent, |version| version.state.clone())
    }

    /// The version visible at checkpoint `seq`.
    pub fn at(&self, path: &str, seq: u64) -> Option<&Version> {
        let versions = self.paths.get(path)?;
        let count = versions.partition_point(|version| version.seq <= seq);
        count.checked_sub(1).map(|index| &versions[index])
    }

    pub fn state_at(&self, path: &str, seq: u64) -> State {
        self.at(path, seq)
            .map_or(State::Absent, |version| version.state.clone())
    }

    pub fn coverage_at(&self, seq: u64) -> bool {
        let Some((at, (complete, _))) = self.coverage.range(..=seq).next_back() else {
            return false;
        };
        *complete
            && self
                .policies
                .range(..=seq)
                .next_back()
                .is_none_or(|(policy, _)| policy <= at)
    }

    /// Coverage belongs to the policy under which it was observed. A newly
    /// adopted scope has unknown coverage until it is actually examined.
    pub fn current_coverage(&self) -> Option<&(bool, String)> {
        let (at, coverage) = self.coverage.iter().next_back()?;
        self.policies
            .keys()
            .next_back()
            .is_none_or(|policy| policy <= at)
            .then_some(coverage)
    }

    pub fn coverage_complete(&self) -> bool {
        self.coverage_at(u64::MAX)
    }

    pub fn policy_at(&self, seq: u64) -> Option<&Policy> {
        self.policies
            .range(..=seq)
            .next_back()
            .map(|(_, (_, policy))| policy)
    }

    pub fn has_baseline(&self) -> bool {
        self.points.values().any(|point| point.kind == "baseline")
    }

    pub fn last_point(&self) -> Option<&Point> {
        self.points.values().next_back()
    }

    /// Whether a reference names retained history.
    pub fn retained(&self, seq: u64) -> bool {
        seq >= self.horizon && seq <= self.latest()
            || self.pins.contains_key(&seq)
            || self.points.contains_key(&seq)
    }

    /// Paths whose version changed in `(after, until]`.
    pub fn changed_between(&self, after: u64, until: u64) -> Vec<&Version> {
        let mut changed: Vec<&Version> = self
            .paths
            .values()
            .filter_map(|versions| {
                versions
                    .iter()
                    .rev()
                    .find(|version| version.seq > after && version.seq <= until)
            })
            .collect();
        changed.sort_by_key(|version| version.seq);
        changed
    }

    /// Paths that exist at `seq`, in path order.
    pub fn existing_at(&self, seq: u64) -> impl Iterator<Item = &Version> {
        self.paths
            .values()
            .filter_map(move |versions| {
                let count = versions.partition_point(|version| version.seq <= seq);
                count.checked_sub(1).map(|index| &versions[index])
            })
            .filter(|version| version.state.exists())
    }
}

pub fn parse_reference(text: &str) -> Result<u64, String> {
    text.trim()
        .strip_prefix("cp-")
        .and_then(|number| number.parse().ok())
        .ok_or_else(|| format!("Invalid checkpoint reference {text:?}; use cp-<number>"))
}

pub fn reference(seq: u64) -> String {
    format!("cp-{seq}")
}
