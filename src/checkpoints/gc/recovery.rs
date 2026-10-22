//! Targeted recovery retention. Live file metadata survives collection, but
//! its object reference survives only through a held recovery or legacy pin.
use super::super::index::Index;
use super::super::record::{Point, Record, Recovery, State, Version};
use super::super::recovery::{self, GROUP_KIND, SNAPSHOT_KIND};
use super::marker;
use std::collections::{BTreeMap, BTreeSet};

fn is_recovery_point(point: &Point) -> bool {
    point.kind == GROUP_KIND || point.kind == SNAPSHOT_KIND
}

fn anchored_versions(index: &Index, horizon: u64, now: u64) -> Vec<&Version> {
    let Some((mode, mode_time)) = index.recovery.mode else {
        return Vec::new();
    };
    let grace = now < mode_time.saturating_add(recovery::LEGACY_GRACE_MS);
    let latest_legacy_point = index
        .points
        .values()
        .filter(|point| point.seq < mode && !is_recovery_point(point))
        .map(|point| point.seq)
        .next_back();
    let recent_milestones = index
        .points
        .values()
        .filter(|point| {
            point.seq >= mode
                && point.kind == "milestone"
                && now < point.time.saturating_add(recovery::MAX_ACCEPTED_MS)
        })
        .map(|point| point.seq);
    let anchors: Vec<u64> = index
        .pins
        .keys()
        .filter(|pin| {
            !index.recovery.groups.contains_key(pin) && !index.recovery.snapshots.contains_key(pin)
        })
        .copied()
        .chain(recent_milestones)
        .collect();
    let mut result = Vec::new();
    for versions in index.paths.values() {
        let legacy: Vec<&Version> = versions.iter().filter(|v| v.seq < mode).collect();
        let mut chosen = BTreeMap::new();
        if grace {
            // A new-format horizon may pass the mode boundary. The latest
            // legacy point still needs its complete path state during grace.
            let boundary = horizon.min(mode.saturating_sub(1));
            let count = legacy.partition_point(|version| version.seq <= boundary);
            if let Some(at) = count.checked_sub(1) {
                chosen.insert(legacy[at].seq, legacy[at]);
            }
            for version in &legacy[count..] {
                chosen.insert(version.seq, *version);
            }
            if let Some(point) = latest_legacy_point {
                let count = legacy.partition_point(|version| version.seq <= point);
                if let Some(at) = count.checked_sub(1) {
                    chosen.insert(legacy[at].seq, legacy[at]);
                }
            }
        }
        for anchor in &anchors {
            let count = versions.partition_point(|version| version.seq <= *anchor);
            if let Some(at) = count.checked_sub(1) {
                chosen.insert(versions[at].seq, &versions[at]);
            }
        }
        result.extend(chosen.into_values());
    }
    result
}

fn scoped_absence(index: &Index, snapshot: u64, path: &str) -> bool {
    let Some(data) = index.recovery.snapshots.get(&snapshot) else {
        return false;
    };
    let policy = index.policy_at(snapshot).cloned().unwrap_or_default();
    data.roots.iter().any(|root| {
        (root.is_empty() || path == root || path.starts_with(&format!("{root}/")))
            && super::super::scan::in_scope(&policy, &[], path, false)
    })
}

fn initial_steps(index: &Index, group: u64) -> BTreeSet<u64> {
    let Some(group) = index.recovery.groups.get(&group) else {
        return BTreeSet::new();
    };
    let mut first = BTreeSet::new();
    let mut seen_paths = BTreeSet::new();
    let mut earlier_scopes = Vec::new();
    for snapshot in &group.snapshots {
        let Some(data) = index.recovery.snapshots.get(snapshot) else {
            continue;
        };
        for (path, step) in &data.steps {
            if seen_paths.insert(path)
                && !earlier_scopes
                    .iter()
                    .any(|earlier| scoped_absence(index, *earlier, path))
            {
                first.insert(step.seq);
            }
        }
        if !data.roots.is_empty() {
            earlier_scopes.push(*snapshot);
        }
    }
    first
}

/// The recovery records whose meaning and object references still matter.
fn selected(index: &Index, now: u64) -> BTreeSet<u64> {
    let mut keep = BTreeSet::new();
    let limits = recovery::limits(index);
    for (group_id, group) in &index.recovery.groups {
        let group_live = recovery::available_at(index, *group_id, now, limits);
        let pinned_snapshots: Vec<u64> = group
            .snapshots
            .iter()
            .copied()
            .filter(|snapshot| index.pins.contains_key(snapshot))
            .collect();
        if !group_live && pinned_snapshots.is_empty() {
            continue;
        }
        let first = if group.accepted.is_some() {
            initial_steps(index, *group_id)
        } else {
            BTreeSet::new()
        };
        keep.insert(*group_id);
        for snapshot_id in &group.snapshots {
            let pinned = index.pins.contains_key(snapshot_id);
            if !group_live && !pinned {
                continue;
            }
            keep.insert(*snapshot_id);
            if let Some(snapshot) = index.recovery.snapshots.get(snapshot_id) {
                for step in snapshot.steps.values() {
                    if pinned
                        || group_live && (group.accepted.is_none() || first.contains(&step.seq))
                    {
                        keep.insert(step.seq);
                    }
                }
            }
        }
    }
    for (seq, record) in &index.recovery.records {
        match record {
            Recovery::Mode { .. } => {
                keep.insert(*seq);
            }
            Recovery::Snapshot {
                group, snapshot, ..
            }
            | Recovery::Scope {
                group, snapshot, ..
            } => {
                if keep.contains(group) && keep.contains(snapshot) {
                    keep.insert(*seq);
                }
            }
            Recovery::Status {
                group,
                action: super::super::record::RecoveryAction::Touched,
                time,
                ..
            } => {
                if keep.contains(group)
                    && index
                        .recovery
                        .groups
                        .get(group)
                        .is_some_and(|data| data.accepted.is_none() && data.last_activity == *time)
                {
                    keep.insert(*seq);
                }
            }
            Recovery::Status { group, .. } => {
                if keep.contains(group) {
                    keep.insert(*seq);
                }
            }
            Recovery::Step { .. } => {}
        }
    }
    keep
}

pub(super) fn due(index: &Index, now: u64) -> bool {
    let Some((mode, mode_time)) = index.recovery.mode else {
        return false;
    };
    let held: BTreeSet<u64> = anchored_versions(index, index.horizon, now)
        .into_iter()
        .map(|version| version.seq)
        .collect();
    if index
        .paths
        .values()
        .filter_map(|versions| versions.last())
        .any(|version| {
            version.seq >= mode
                && matches!(version.state, State::File { .. })
                && !held.contains(&version.seq)
        })
    {
        return true;
    }
    let selected = selected(index, now);
    if index
        .recovery
        .records
        .keys()
        .any(|seq| !selected.contains(seq))
    {
        return true;
    }
    if index.points.values().any(|point| {
        !is_recovery_point(point)
            && !index.pins.contains_key(&point.seq)
            && if point.seq < mode {
                now >= mode_time.saturating_add(recovery::LEGACY_GRACE_MS)
            } else {
                now >= point.time.saturating_add(recovery::MAX_ACCEPTED_MS)
            }
    }) {
        return true;
    }
    if now >= mode_time.saturating_add(recovery::LEGACY_GRACE_MS)
        && index
            .paths
            .values()
            .flat_map(|versions| versions.iter())
            .any(|version| {
                version.seq < mode
                    && matches!(version.state, State::File { .. })
                    && !held.contains(&version.seq)
            })
    {
        return true;
    }
    false
}

pub(super) fn snapshot(index: &Index, horizon: u64, now: u64) -> Vec<Record> {
    let Some((mode, mode_time)) = index.recovery.mode else {
        return Vec::new();
    };
    let selected = selected(index, now);
    let grace = now < mode_time.saturating_add(recovery::LEGACY_GRACE_MS);
    let mut records = Vec::new();
    let mut held_versions = BTreeSet::new();
    for version in anchored_versions(index, horizon, now) {
        held_versions.insert(version.seq);
        records.push(Record::Version(version.clone()));
    }
    for versions in index.paths.values() {
        if let Some(current) = versions.last() {
            if held_versions.contains(&current.seq) {
                continue;
            }
            let mut head = current.clone();
            if let State::File {
                length,
                digest,
                mode,
                ..
            } = head.state
            {
                head.state = State::UnretainedFile {
                    length,
                    digest,
                    mode,
                };
            }
            records.push(Record::Version(head));
        }
    }
    for (path, stat) in &index.stats {
        if let Some(current) = index.current(path)
            && current.stat.as_ref() != Some(stat)
        {
            records.push(Record::Stat {
                seq: current.seq,
                path: path.clone(),
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
    let latest_legacy = index
        .points
        .values()
        .filter(|point| point.seq < mode && !is_recovery_point(point))
        .map(|point| point.seq)
        .next_back()
        .filter(|_| grace);
    records.extend(
        index
            .points
            .values()
            .filter(|point| {
                selected.contains(&point.seq)
                    || !is_recovery_point(point)
                        && (index.pins.contains_key(&point.seq)
                            || point.seq < mode
                                && grace
                                && (point.seq >= horizon || latest_legacy == Some(point.seq))
                            || point.seq >= mode
                                && point.kind == "milestone"
                                && now < point.time.saturating_add(recovery::MAX_ACCEPTED_MS))
            })
            .cloned()
            .map(Record::Point),
    );
    records.extend(
        index
            .recovery
            .records
            .iter()
            .filter(|(seq, _)| selected.contains(seq))
            .map(|(_, record)| Record::Recovery(record.clone())),
    );
    records.extend(index.pins.iter().map(|(target, label)| Record::Pin {
        seq: *target,
        target: *target,
        pinned: true,
        label: label.clone(),
    }));
    // Policy and coverage records are small, and earlier pinned points need
    // the rules in force when their states were captured.
    records.extend(
        index
            .policies
            .iter()
            .map(|(seq, (id, policy))| Record::Policy {
                seq: *seq,
                id: *id,
                text: policy.encode(),
            }),
    );
    records.extend(
        index
            .coverage
            .iter()
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

#[cfg(test)]
mod tests;
