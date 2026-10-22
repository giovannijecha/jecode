//! Best-effort compaction of closed, current-format archives during the
//! hourly sweep. The probe reads only archive metadata and journal records.
use super::*;
use crate::checkpoints::{gc, recovery};

/// Whole-archive inactivity cannot discard a newer, still-live recovery
/// group, even if the archive's last user-access timestamp is old.
pub(super) fn held(index: &Index, now: u64) -> bool {
    let Some((mode, mode_time)) = index.recovery.mode else {
        return false;
    };
    if now < mode_time.saturating_add(recovery::LEGACY_GRACE_MS) {
        return true;
    }
    let limits = recovery::limits(index);
    index
        .recovery
        .groups
        .keys()
        .any(|group| recovery::available_at(index, *group, now, limits))
        || index.points.values().any(|point| {
            point.seq >= mode
                && point.kind == "milestone"
                && now < point.time.saturating_add(recovery::MAX_ACCEPTED_MS)
        })
}

/// Returns false for busy or damaged archives; the next sweep can retry.
pub(super) fn due(archive: &Path) -> bool {
    probe(archive).unwrap_or(false)
}

fn probe(archive: &Path) -> Result<bool, String> {
    if archive.join(".purging").exists()
        || archive.join(".migrating").exists()
        || expiry_pending_path(archive).exists()
        || !has_history(archive)?
        || (!layout::current(archive).exists()
            && !layout::generation_directory(archive, 1)
                .join("000001.log")
                .is_file())
    {
        return Ok(false);
    }
    let Some(_store) = Exclusive::try_acquire(&layout::store_lock(archive))
        .map_err(|error| format!("Could not lock checkpoint archive: {error}"))?
    else {
        return Ok(false);
    };
    let mut journal = Journal::open(archive)?;
    let mut index = Index::default();
    let mut followed = HashSet::new();
    loop {
        journal.catch_up(&mut |entry| index.apply(entry))?;
        if !journal.damage.is_empty() || journal.blocked.is_some() {
            return Ok(false);
        }
        let Some(generation) = index.moved else {
            break;
        };
        if !followed.insert(generation) {
            return Ok(false);
        }
        journal = Journal::open_existing_generation(archive, generation)?;
        index = Index::default();
    }
    Ok(index.recovery.mode.is_some() && gc::due(&index, crate::checkpoints::clock::now()))
}

/// Called after the sweeper releases its exclusive lifecycle lease. Opening
/// this Store must never renew last-use or inspect the workspace itself.
pub(super) fn collect(archive: &Path) -> Result<(), String> {
    let text = fs::read_to_string(archive.join("workspace.json"))
        .map_err(|error| format!("Could not read checkpoint identity: {error}"))?;
    let value = crate::json::parse(&text)?;
    let root = value
        .get("root")
        .and_then(crate::json::Value::as_str)
        .map(PathBuf::from)
        .filter(|root| root.is_absolute())
        .ok_or_else(|| "Invalid checkpoint workspace identity".to_string())?;
    gc::collect(
        &Location {
            root,
            directory: archive.to_path_buf(),
        },
        false,
    )?;
    Ok(())
}
