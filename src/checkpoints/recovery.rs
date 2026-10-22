//! Journal-backed, path-specific recovery groups. A snapshot names the exact
//! state before one tool step; an accepted group keeps the first state for
//! each path. Neither meaning depends on a global workspace scan.
use super::index::Index;
use super::record::{Point, Record, Recovery, RecoveryAction, State};
use super::store::Transaction;
use std::collections::{BTreeMap, BTreeSet};

pub const GROUP_KIND: &str = "recovery-group";
pub const SNAPSHOT_KIND: &str = "recovery-before";
const DAY: u64 = 24 * 60 * 60 * 1_000;
pub const MAX_ACCEPTED_MS: u64 = DAY;
pub const LEGACY_GRACE_MS: u64 = 7 * DAY;

#[derive(Clone, Copy, Debug)]
pub struct Expiry {
    pub accepted_ms: u64,
    pub active_ms: Option<u64>,
}

impl Default for Expiry {
    fn default() -> Self {
        Self {
            accepted_ms: DAY,
            active_ms: Some(7 * DAY),
        }
    }
}

impl Expiry {
    pub fn accepted(self) -> u64 {
        self.accepted_ms.min(MAX_ACCEPTED_MS)
    }
}

#[derive(Clone, Debug)]
pub struct Step {
    pub seq: u64,
    pub before: State,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub group: u64,
    pub time: u64,
    pub steps: BTreeMap<String, Step>,
    pub roots: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub origin: String,
    pub created: u64,
    pub last_activity: u64,
    pub accepted: Option<u64>,
    pub accepted_until: Option<u64>,
    pub snapshots: Vec<u64>,
}

#[derive(Clone, Default)]
pub struct RecoveryIndex {
    pub mode: Option<(u64, u64)>,
    pub groups: BTreeMap<u64, Group>,
    pub snapshots: BTreeMap<u64, Snapshot>,
    pub records: BTreeMap<u64, Recovery>,
}

impl RecoveryIndex {
    pub fn point(&mut self, point: &Point) {
        if point.kind == GROUP_KIND {
            self.groups.insert(
                point.seq,
                Group {
                    origin: point.origin.clone(),
                    created: point.time,
                    last_activity: point.time,
                    accepted: None,
                    accepted_until: None,
                    snapshots: Vec::new(),
                },
            );
        }
    }

    pub fn record(&mut self, record: Recovery, points: &BTreeMap<u64, Point>) {
        match &record {
            Recovery::Mode { seq, time } => self.mode = Some((*seq, *time)),
            Recovery::Snapshot {
                group, snapshot, ..
            } => {
                if let (Some(group_data), Some(point)) =
                    (self.groups.get_mut(group), points.get(snapshot))
                {
                    group_data.snapshots.push(*snapshot);
                    group_data.last_activity = group_data.last_activity.max(point.time);
                    self.snapshots.insert(
                        *snapshot,
                        Snapshot {
                            group: *group,
                            time: point.time,
                            steps: BTreeMap::new(),
                            roots: BTreeSet::new(),
                        },
                    );
                }
            }
            Recovery::Step {
                group,
                snapshot,
                path,
                before,
                ..
            } => {
                if let Some(snapshot) = self
                    .snapshots
                    .get_mut(snapshot)
                    .filter(|snapshot| snapshot.group == *group)
                {
                    snapshot.steps.entry(path.clone()).or_insert_with(|| Step {
                        seq: record.seq(),
                        before: before.clone(),
                    });
                }
            }
            Recovery::Scope {
                group,
                snapshot,
                root,
                ..
            } => {
                if let Some(snapshot) = self
                    .snapshots
                    .get_mut(snapshot)
                    .filter(|snapshot| snapshot.group == *group)
                {
                    snapshot.roots.insert(root.clone());
                }
            }
            Recovery::Status {
                group,
                action: RecoveryAction::Accepted,
                time,
                until,
                ..
            } => {
                if let Some(group) = self.groups.get_mut(group)
                    && group.accepted.is_none()
                {
                    group.accepted = Some(*time);
                    group.accepted_until = Some((*until).min(time.saturating_add(MAX_ACCEPTED_MS)));
                }
            }
            Recovery::Status {
                group,
                action: RecoveryAction::Touched,
                time,
                ..
            } => {
                if let Some(group) = self.groups.get_mut(group) {
                    group.last_activity = group.last_activity.max(*time);
                }
            }
        }
        self.records.insert(record.seq(), record);
    }
}

/// Starts the targeted format before the first targeted Version is recorded.
pub fn enable(tx: &mut Transaction<'_>) -> Result<(), String> {
    if tx.index().recovery.mode.is_none() {
        let seq = tx.seq();
        tx.record(Record::Recovery(Recovery::Mode {
            seq,
            time: crate::checkpoints::clock::now(),
        }));
        tx.flush(false)?;
    }
    Ok(())
}

pub fn begin_snapshot(
    tx: &mut Transaction<'_>,
    origin: &str,
    label: &str,
) -> Result<(u64, u64), String> {
    enable(tx)?;
    let group = match group_for(tx.index(), origin) {
        Some(group) => group,
        None => tx.point(GROUP_KIND, label, origin, false),
    };
    let snapshot = tx.point(SNAPSHOT_KIND, label, origin, false);
    let seq = tx.seq();
    tx.record(Record::Recovery(Recovery::Snapshot {
        seq,
        group,
        snapshot,
    }));
    Ok((group, snapshot))
}

pub fn add(
    tx: &mut Transaction<'_>,
    group: u64,
    snapshot: u64,
    path: &str,
    before: &State,
) -> Result<(), String> {
    check_snapshot(tx.index(), group, snapshot)?;
    if path.is_empty() || !before.recoverable() {
        return Err("Recovery step needs a path with a recoverable before-state".into());
    }
    let seq = tx.seq();
    tx.record(Record::Recovery(Recovery::Step {
        seq,
        group,
        snapshot,
        path: path.into(),
        before: before.clone(),
    }));
    Ok(())
}

/// Marks a root only after every existing recoverable descendant was saved.
pub fn scope(
    tx: &mut Transaction<'_>,
    group: u64,
    snapshot: u64,
    root: &str,
) -> Result<(), String> {
    check_snapshot(tx.index(), group, snapshot)?;
    let seq = tx.seq();
    tx.record(Record::Recovery(Recovery::Scope {
        seq,
        group,
        snapshot,
        root: root.into(),
    }));
    Ok(())
}

pub fn accept(tx: &mut Transaction<'_>, group: u64, origin: &str) -> Result<(), String> {
    let data = tx
        .index()
        .recovery
        .groups
        .get(&group)
        .ok_or("Unknown recovery group")?;
    if data.origin != origin {
        return Err("Recovery group belongs to another conversation".into());
    }
    if data.accepted.is_some() {
        return Ok(());
    }
    if !available(tx.index(), group) {
        return Err("Recovery group has expired".into());
    }
    let now = crate::checkpoints::clock::now();
    let until = now.saturating_add(limits(tx.index()).accepted());
    let seq = tx.seq();
    tx.record(Record::Recovery(Recovery::Status {
        seq,
        group,
        action: RecoveryAction::Accepted,
        time: now,
        until,
    }));
    Ok(())
}

/// Renews a running persistent command's inactivity window. A crashed
/// monitor writes no more touches, so the group still expires after 7 days.
pub fn touch(tx: &mut Transaction<'_>, group: u64, origin: &str) -> Result<(), String> {
    const TOUCH_INTERVAL: u64 = 60 * 60 * 1_000;
    let data = tx
        .index()
        .recovery
        .groups
        .get(&group)
        .ok_or("Unknown recovery group")?;
    if data.origin != origin {
        return Err("Recovery group belongs to another conversation".into());
    }
    if !available(tx.index(), group) {
        return Err("Recovery group is no longer active".into());
    }
    if data.accepted.is_some() {
        return Ok(());
    }
    let now = crate::checkpoints::clock::now();
    if now.saturating_sub(data.last_activity) < TOUCH_INTERVAL {
        return Ok(());
    }
    let seq = tx.seq();
    tx.record(Record::Recovery(Recovery::Status {
        seq,
        group,
        action: RecoveryAction::Touched,
        time: now,
        until: 0,
    }));
    Ok(())
}

fn check_snapshot(index: &Index, group: u64, snapshot: u64) -> Result<(), String> {
    let group_data = index
        .recovery
        .groups
        .get(&group)
        .ok_or("Unknown recovery group")?;
    if group_data.accepted.is_some() {
        return Err("Recovery group was already accepted".into());
    }
    if index.recovery.snapshots.get(&snapshot).map(|s| s.group) != Some(group) {
        return Err("Recovery snapshot does not belong to this group".into());
    }
    if !available(index, group) {
        return Err("Recovery group has expired".into());
    }
    Ok(())
}

pub fn group_for(index: &Index, origin: &str) -> Option<u64> {
    index.recovery.groups.iter().rev().find_map(|(id, group)| {
        (group.origin == origin && group.accepted.is_none() && available(index, *id)).then_some(*id)
    })
}

fn in_root(path: &str, root: &str) -> bool {
    root.is_empty() || path == root || path.starts_with(&format!("{root}/"))
}

fn snapshot_target(
    index: &Index,
    snapshot_id: u64,
    snapshot: &Snapshot,
    path: &str,
) -> Option<State> {
    snapshot
        .steps
        .get(path)
        .map(|step| step.before.clone())
        .or_else(|| {
            let policy = index.policy_at(snapshot_id).cloned().unwrap_or_default();
            if snapshot.roots.iter().any(|root| in_root(path, root))
                && super::scan::in_scope(&policy, &[], path, false)
            {
                Some(State::Absent)
            } else {
                None
            }
        })
}

pub fn target(index: &Index, reference: u64, path: &str) -> Option<State> {
    if !available(index, reference) {
        return None;
    }
    if let Some(snapshot) = index.recovery.snapshots.get(&reference) {
        return snapshot_target(index, reference, snapshot, path);
    }
    let group = index.recovery.groups.get(&reference)?;
    group
        .snapshots
        .iter()
        .filter_map(|id| {
            index
                .recovery
                .snapshots
                .get(id)
                .map(|snapshot| (*id, snapshot))
        })
        .find_map(|(id, snapshot)| snapshot_target(index, id, snapshot, path))
}

pub fn roots(index: &Index, reference: u64) -> Vec<String> {
    if !available(index, reference) {
        return Vec::new();
    }
    if let Some(snapshot) = index.recovery.snapshots.get(&reference) {
        return snapshot.roots.iter().cloned().collect();
    }
    index
        .recovery
        .groups
        .get(&reference)
        .into_iter()
        .flat_map(|group| &group.snapshots)
        .filter_map(|id| index.recovery.snapshots.get(id))
        .flat_map(|snapshot| snapshot.roots.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Explicit paths captured by a snapshot or any step of a group. Complete
/// roots are returned separately because their absent descendants are open
/// ended until restore examines the live workspace.
pub fn paths(index: &Index, reference: u64) -> Vec<String> {
    if !available(index, reference) {
        return Vec::new();
    }
    if let Some(snapshot) = index.recovery.snapshots.get(&reference) {
        return snapshot.steps.keys().cloned().collect();
    }
    index
        .recovery
        .groups
        .get(&reference)
        .into_iter()
        .flat_map(|group| &group.snapshots)
        .filter_map(|id| index.recovery.snapshots.get(id))
        .flat_map(|snapshot| snapshot.steps.keys().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn available(index: &Index, reference: u64) -> bool {
    available_at(
        index,
        reference,
        crate::checkpoints::clock::now(),
        limits(index),
    )
}

pub fn retained(index: &Index, reference: u64) -> bool {
    available(index, reference)
}

pub fn available_at(index: &Index, reference: u64, now: u64, limits: Expiry) -> bool {
    if let Some(group) = index.recovery.groups.get(&reference) {
        return index.pins.contains_key(&reference)
            || deadline(group, limits).is_none_or(|deadline| now < deadline);
    }
    let Some(snapshot) = index.recovery.snapshots.get(&reference) else {
        return false;
    };
    if index.pins.contains_key(&reference) {
        return true;
    }
    let Some(group) = index.recovery.groups.get(&snapshot.group) else {
        return false;
    };
    group.accepted.is_none() && available_at(index, snapshot.group, now, limits)
}

pub fn deadline(group: &Group, limits: Expiry) -> Option<u64> {
    match group.accepted {
        Some(time) => Some(
            group
                .accepted_until
                .unwrap_or_else(|| time.saturating_add(limits.accepted()))
                .min(time.saturating_add(MAX_ACCEPTED_MS)),
        ),
        None => limits
            .active_ms
            .map(|duration| group.last_activity.saturating_add(duration)),
    }
}

pub fn limits(index: &Index) -> Expiry {
    index
        .policy_at(u64::MAX)
        .map(super::policy::Policy::recovery_expiry)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
