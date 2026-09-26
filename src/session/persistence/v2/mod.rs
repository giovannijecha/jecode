//! Incremental canonical log; the small head is its commit record.
//! This coordinator keeps creation, append tracking, recovery and verified
//! import together because they share one commit cursor and lease invariant.
//! Frame I/O, head encoding and replay have separate bounded modules.
#[cfg(test)]
mod batch_tests;
mod head;
mod log;
#[cfg(test)]
mod provider_failure_tests;
#[cfg(test)]
mod recall_tests;
mod replay;
#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod traversal_tests;

use super::{
    CanonicalCursor, CanonicalPage, CanonicalSlice, Listed, Model, Record, Saved, Store, codec,
    invalid, now, text, valid_id,
};
use crate::{
    json::{self, Value},
    session::history::{History, Turn},
    workspace::{Access, Workspace},
};
use std::{
    collections::BTreeMap,
    io,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

#[derive(Clone, Default)]
struct StepMark {
    core: u64,
    receipts: Vec<u64>,
}
#[derive(Clone)]
pub(super) struct Tracker {
    pub(super) committed: u64,
    pub(super) rolling: u64,
    pub(super) turns: usize,
    pub(super) title: String,
    pub(super) recent: Vec<String>,
    verified: bool,
    /// A head replacement may have committed even when its final OS sync
    /// reports an error. A fresh lease must inspect the head before writing.
    uncertain_commit: bool,
    #[cfg(test)]
    fail_after_head_replace: bool,
    step_base: usize,
    guidance_base: usize,
    steps: Vec<StepMark>,
    guidance: Vec<u64>,
    end: Option<u64>,
}
impl Default for Tracker {
    fn default() -> Self {
        Self {
            committed: 0,
            rolling: log::HASH_START,
            turns: 0,
            title: String::new(),
            recent: Vec::new(),
            verified: true,
            uncertain_commit: false,
            #[cfg(test)]
            fail_after_head_replace: false,
            step_base: 0,
            guidance_base: 0,
            steps: Vec::new(),
            guidance: Vec::new(),
            end: None,
        }
    }
}

pub(super) fn create(
    root: &Store,
    model: Model,
    directory: Option<&Path>,
    workspace: Option<&Workspace>,
) -> io::Result<History> {
    create_with_verification(root, model, directory, workspace, true)
}
pub(super) fn create_unverified(
    root: &Store,
    model: Model,
    directory: Option<&Path>,
    workspace: Option<&Workspace>,
) -> io::Result<History> {
    create_with_verification(root, model, directory, workspace, false)
}
fn create_with_verification(
    root: &Store,
    model: Model,
    directory: Option<&Path>,
    workspace: Option<&Workspace>,
    verified: bool,
) -> io::Result<History> {
    let settings = crate::state::settings::Settings::load(root)?;
    let store = root.directory("sessions-v2")?;
    static NEXT: AtomicU64 = AtomicU64::new(0x80000000);
    let created = now()?;
    let id = format!(
        "s-{created:016x}-{:08x}-{:08x}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let lock = store.lock(
        &format!("{id}.lock"),
        &AtomicBool::new(false),
        Instant::now(),
    )?;
    if store
        .read(&format!("{id}.head"), head::HEAD_LIMIT)?
        .is_some()
    {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    log::open(&store, &id, true)?.sync_all()?;
    store.sync_root()?;
    let record = Record {
        store,
        id,
        model,
        workspace: workspace
            .map(|w| w.path().to_str().map(str::to_owned).ok_or_else(invalid))
            .transpose()?,
        directory: directory
            .map(|p| p.to_str().map(str::to_owned).ok_or_else(invalid))
            .transpose()?,
        access: workspace.map_or(Access::Workspace, Workspace::access),
        created,
        extra: BTreeMap::new(),
        incremental: Some(Mutex::new(Tracker {
            verified,
            ..Tracker::default()
        })),
        _lock: lock,
    };
    let mut history = History {
        record: Some(record),
        ..Default::default()
    };
    history.projection.limit_bytes = settings.context_limit_bytes;
    history
        .checkpoint()
        .map_err(|_| io::Error::other("cannot commit new session head"))?;
    Ok(history)
}

fn envelope(kind: &str, step: usize, index: usize, data: Value) -> Value {
    json::object([
        ("kind", text(kind)),
        ("step", Value::Number(step.to_string())),
        ("index", Value::Number(index.to_string())),
        ("data", data),
    ])
}
fn emit(
    file: &mut std::fs::File,
    tracker: &mut Tracker,
    turn: usize,
    value: &Value,
) -> io::Result<()> {
    log::append(
        file,
        turn,
        value,
        &mut tracker.committed,
        &mut tracker.rolling,
    )
}
fn begin_value(prompt: &str) -> Value {
    json::object([
        ("prompt", text(prompt)),
        ("outcome", text("")),
        ("end", text("active")),
        ("metrics", codec::metrics(&Default::default())),
        ("steps", Value::Array(Vec::new())),
        ("guidance", Value::Array(Vec::new())),
    ])
}

fn sync_turn(
    file: &mut std::fs::File,
    tracker: &mut Tracker,
    turn_index: usize,
    step_base: usize,
    guidance_base: usize,
    turn: &Turn,
) -> io::Result<()> {
    for (index, step) in turn.steps.iter().enumerate() {
        if !codec::valid_incremental_step(step) {
            return Err(invalid());
        }
        let core = codec::step_core(step);
        let digest = log::fingerprint(&core)?;
        if tracker.steps.len() == index {
            emit(
                file,
                tracker,
                turn_index,
                &envelope("step", step_base + index, 0, core),
            )?;
            tracker.steps.push(StepMark {
                core: digest,
                receipts: Vec::new(),
            });
        } else if tracker.steps[index].core != digest {
            emit(
                file,
                tracker,
                turn_index,
                &envelope("step", step_base + index, 0, core),
            )?;
            tracker.steps[index].core = digest;
        }
        if tracker.steps[index].receipts.len() > step.results.len() {
            return Err(invalid());
        }
        for (receipt_index, receipt) in step.results.iter().enumerate() {
            let value = codec::receipt(receipt);
            let digest = log::fingerprint(&value)?;
            if tracker.steps[index].receipts.get(receipt_index) != Some(&digest) {
                emit(
                    file,
                    tracker,
                    turn_index,
                    &envelope("receipt", step_base + index, receipt_index, value),
                )?;
                if tracker.steps[index].receipts.len() == receipt_index {
                    tracker.steps[index].receipts.push(digest);
                } else {
                    tracker.steps[index].receipts[receipt_index] = digest;
                }
            }
        }
    }
    if tracker.steps.len() > turn.steps.len() || tracker.guidance.len() > turn.guidance.len() {
        return Err(invalid());
    }
    for (index, guidance) in turn.guidance.iter().enumerate() {
        let mut value = codec::guidance(guidance);
        if let Value::Object(fields) = &mut value {
            fields.insert(
                "after_step".into(),
                Value::Number((step_base + guidance.after_step).to_string()),
            );
        }
        let digest = log::fingerprint(&value)?;
        if tracker.guidance.get(index) != Some(&digest) {
            emit(
                file,
                tracker,
                turn_index,
                &envelope("guidance", 0, guidance_base + index, value),
            )?;
            if tracker.guidance.len() == index {
                tracker.guidance.push(digest);
            } else {
                tracker.guidance[index] = digest;
            }
        }
    }
    let value = codec::end(turn);
    let digest = log::fingerprint(&value)?;
    if tracker.end != Some(digest) {
        emit(file, tracker, turn_index, &envelope("end", 0, 0, value))?;
        tracker.end = Some(digest);
    }
    Ok(())
}

pub(super) fn save(record: &Record, history: &History) -> io::Result<()> {
    let guard = record.incremental.as_ref().ok_or_else(invalid)?;
    let mut guard = guard
        .lock()
        .map_err(|_| io::Error::other("session log lock poisoned"))?;
    if guard.uncertain_commit {
        return Err(io::Error::other(
            "session head commit is uncertain; close and resume before writing",
        ));
    }
    let mut next = guard.clone();
    let total = history.turn_count();
    if total < next.turns || history.base_turn > next.turns {
        return Err(invalid());
    }
    let mut file = log::open(&record.store, &record.id, false)?;
    log::prepare(&mut file, next.committed)?;
    for turn_index in next.turns..total {
        let turn = history
            .turns
            .get(turn_index - history.base_turn)
            .ok_or_else(invalid)?;
        emit(
            &mut file,
            &mut next,
            turn_index,
            &envelope("begin", 0, 0, begin_value(&turn.prompt)),
        )?;
        if next.title.is_empty() {
            next.title = turn.prompt.clone();
        }
        next.recent.push(turn.prompt.clone());
        if next.recent.len() > super::super::MAX_RECALLED_PROMPTS {
            next.recent.remove(0);
        }
        next.steps.clear();
        next.step_base = 0;
        next.guidance_base = 0;
        next.guidance.clear();
        next.end = None;
        sync_turn(&mut file, &mut next, turn_index, 0, 0, turn)?;
        next.turns += 1;
    }
    if total != 0 && next.turns == total && history.base_turn < total {
        let turn = history.turns.last().ok_or_else(invalid)?;
        let step_base = if history.base_turn == total - 1 {
            history.base_step
        } else {
            0
        };
        if step_base < next.step_base || step_base - next.step_base > next.steps.len() {
            return Err(invalid());
        }
        next.steps.drain(..step_base - next.step_base);
        next.step_base = step_base;
        let guidance_base = if history.base_turn == total - 1 {
            history.base_guidance
        } else {
            0
        };
        if guidance_base < next.guidance_base
            || guidance_base - next.guidance_base > next.guidance.len()
        {
            return Err(invalid());
        }
        next.guidance.drain(..guidance_base - next.guidance_base);
        next.guidance_base = guidance_base;
        sync_turn(
            &mut file,
            &mut next,
            total - 1,
            step_base,
            guidance_base,
            turn,
        )?;
    }
    file.sync_all()?;
    let head_result = head::write(record, history, &mut next);
    #[cfg(test)]
    let head_result = if next.fail_after_head_replace && head_result.is_ok() {
        Err(io::Error::other("injected failure after head replacement"))
    } else {
        head_result
    };
    if let Err(error) = head_result {
        guard.uncertain_commit = true;
        return Err(error);
    }
    *guard = next;
    Ok(())
}

pub(super) fn load(root: &Store, id: &str, leased: bool) -> io::Result<Saved> {
    load_inner(root, id, leased, false, leased)
}
pub(super) fn inspect_leased(root: &Store, id: &str) -> io::Result<Saved> {
    load_inner(root, id, true, false, false)
}
pub(super) fn load_unverified(root: &Store, id: &str) -> io::Result<Saved> {
    load_inner(root, id, false, true, false)
}
fn load_inner(
    root: &Store,
    id: &str,
    leased: bool,
    allow_unverified: bool,
    recover_interruption: bool,
) -> io::Result<Saved> {
    if !valid_id(id) {
        return Err(invalid());
    }
    let store = root.directory("sessions-v2")?;
    let lock = if leased {
        Some(store.lock(
            &format!("{id}.lock"),
            &AtomicBool::new(false),
            Instant::now(),
        )?)
    } else {
        None
    };
    let info = checked_head(&store, id)?;
    if !info.verified && !allow_unverified {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "session import has not been verified",
        ));
    }
    let first = info.through;
    let mut replay = replay::Replay::new(first, info.step, info.guidance_base);
    log::visit(
        &store,
        id,
        info.committed,
        info.rolling,
        first,
        usize::MAX,
        |turn, value| replay.apply(turn, value),
    )?;
    let mut history = replay.finish()?;
    if first + history.turns.len() != info.turns {
        return Err(log::corrupt());
    }
    history.base_turn = first;
    history.base_step = info.step;
    history.base_guidance = info.guidance_base;
    head::restore_projection(&mut history, &info)?;
    if !super::super::context::valid_cursor(&history) {
        return Err(log::corrupt());
    }
    let title = info.title.clone();
    let turns = info.turns;
    let mut tracker = Tracker {
        committed: info.committed,
        rolling: info.rolling,
        turns,
        title: title.clone(),
        recent: info.recent,
        verified: info.verified,
        step_base: if turns == first + 1 { info.step } else { 0 },
        guidance_base: if turns == first + 1 {
            info.guidance_base
        } else {
            0
        },
        ..Tracker::default()
    };
    if let Some(turn) = history.turns.last() {
        tracker.steps = turn
            .steps
            .iter()
            .map(|step| {
                Ok(StepMark {
                    core: log::fingerprint(&codec::step_core(step))?,
                    receipts: step
                        .results
                        .iter()
                        .map(|receipt| log::fingerprint(&codec::receipt(receipt)))
                        .collect::<io::Result<Vec<_>>>()?,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        let step_base = tracker.step_base;
        tracker.guidance = turn
            .guidance
            .iter()
            .map(|g| {
                let mut value = codec::guidance(g);
                if let Value::Object(fields) = &mut value {
                    fields.insert(
                        "after_step".into(),
                        Value::Number((step_base + g.after_step).to_string()),
                    );
                }
                log::fingerprint(&value)
            })
            .collect::<io::Result<Vec<_>>>()?;
        tracker.end = Some(log::fingerprint(&codec::end(turn))?);
    }
    if let Some(lock) = lock {
        let mut recovered_interruption = false;
        if recover_interruption
            && let Some(turn) = history.turns.last_mut()
            && turn.end.is_none()
        {
            turn.end = Some(super::super::End::Failed(super::super::Failure::Worker));
            turn.outcome = "Interrupted session / no operation was replayed. Verify any unknown tool outcome before continuing.".into();
            recovered_interruption = true;
        }
        history.record = Some(Record {
            store,
            id: id.into(),
            model: info.model,
            workspace: info
                .workspace
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            directory: info
                .directory
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            access: info.access,
            created: info.created,
            extra: BTreeMap::new(),
            incremental: Some(Mutex::new(tracker)),
            _lock: lock,
        });
        // Commit the recovered outcome before a later prompt can append a new
        // turn. Replay requires every earlier turn to have an ended state.
        if recovered_interruption {
            history.checkpoint().map_err(|_| {
                io::Error::other("cannot commit interrupted session outcome on resume")
            })?;
        }
    }
    Ok(Saved {
        id: id.into(),
        model: info.model,
        workspace: info.workspace,
        directory: info.directory,
        access: info.access,
        title,
        turns,
        history,
    })
}

pub(super) fn mark_verified(history: &History) -> io::Result<()> {
    let record = history.record.as_ref().ok_or_else(invalid)?;
    let tracker = record.incremental.as_ref().ok_or_else(invalid)?;
    {
        let mut tracker = tracker.lock().map_err(|_| invalid())?;
        tracker.verified = true;
    }
    record.save(history)
}

pub(super) fn verify_import(
    root: &Store,
    id: &str,
    source: &Saved,
    expected: &Value,
    directory: &Path,
    workspace: Option<&Path>,
) -> io::Result<()> {
    let saved = load_unverified(root, id)?;
    if saved.model != source.model
        || saved.access != source.access
        || saved.directory.as_deref() != Some(directory)
        || saved.workspace.as_deref() != workspace
        || saved.turns != source.history.turn_count()
    {
        return Err(io::Error::other("import metadata verification failed"));
    }
    let info = head::read(&root.directory("sessions-v2")?, id)?;
    if info.projection != head::projection(&source.history) {
        return Err(io::Error::other("import projection verification failed"));
    }
    let turns = expected.array().ok_or_else(invalid)?;
    // The unleased read has no attached record; use a bounded page directly.
    let store = root.directory("sessions-v2")?;
    for start in (0..turns.len()).step_by(16) {
        let end = (start + 16).min(turns.len());
        let mut replay = replay::Replay::limited(start, 0, 0, 80 * 1024 * 1024);
        log::visit(
            &store,
            id,
            info.committed,
            info.rolling,
            start,
            end,
            |turn, value| replay.apply(turn, value),
        )?;
        let page = replay.finish()?.turns;
        if page.len() != end - start {
            return Err(log::corrupt());
        }
        for (offset, turn) in page.iter().enumerate() {
            if codec::encode_turn(turn) != turns[start + offset] {
                return Err(io::Error::other(
                    "import canonical equivalence verification failed",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn has_head(root: &Store, id: &str) -> io::Result<bool> {
    if !valid_id(id) {
        return Err(invalid());
    }
    match root
        .directory("sessions-v2")?
        .read_file(&format!("{id}.head"))
    {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn listed(root: &Store) -> io::Result<Vec<(std::time::SystemTime, String)>> {
    let store = root.directory("sessions-v2")?;
    Ok(store
        .names()?
        .into_iter()
        .filter_map(|name| {
            let id = name.strip_suffix(".head")?;
            valid_id(id).then(|| {
                let modified = std::fs::symlink_metadata(store.root().join(&name))
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                (modified, id.to_owned())
            })
        })
        .collect())
}
fn checked_head(store: &Store, id: &str) -> io::Result<head::Info> {
    head::read(store, id).map_err(|error| {
        if error.kind() == io::ErrorKind::Other {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "committed session head is corrupt",
            )
        } else {
            error
        }
    })
}

pub(super) fn overview(
    root: &Store,
    id: &str,
    modified: std::time::SystemTime,
) -> io::Result<Listed> {
    let info = checked_head(&root.directory("sessions-v2")?, id)?;
    if !info.verified {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "session import has not been verified",
        ));
    }
    Ok(Listed {
        id: id.into(),
        model: Some(info.model),
        title: info.title,
        turns: info.turns,
        workspace: info.workspace,
        directory: info.directory,
        modified,
    })
}

/// Traverse a bounded page without materializing earlier or later turns.
pub(super) fn page(record: &Record, start: usize, count: usize) -> io::Result<Vec<Turn>> {
    if count > 16 {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let info = checked_head(&record.store, &record.id)?;
    if start > info.turns {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let end = start.saturating_add(count).min(info.turns);
    let mut replay = replay::Replay::limited(start, 0, 0, 80 * 1024 * 1024);
    log::visit(
        &record.store,
        &record.id,
        info.committed,
        info.rolling,
        start,
        end,
        |turn, value| replay.apply(turn, value),
    )?;
    let turns = replay.finish()?.turns;
    if start + turns.len() != end {
        return Err(log::corrupt());
    }
    Ok(turns)
}

pub(super) fn turn_slices(
    record: &Record,
    turn: usize,
    cursor: Option<CanonicalCursor>,
    max_bytes: usize,
) -> io::Result<CanonicalPage> {
    let info = checked_head(&record.store, &record.id)?;
    if turn >= info.turns
        || cursor.is_some_and(|cursor| {
            cursor.turn != turn
                || cursor.committed != info.committed
                || cursor.rolling != info.rolling
        })
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let offset = cursor.map_or(0, |cursor| cursor.offset);
    let page = log::chunks(
        &record.store,
        &record.id,
        info.committed,
        info.rolling,
        turn,
        offset,
        max_bytes,
    )?;
    Ok(CanonicalPage {
        slices: page
            .slices
            .into_iter()
            .map(|slice| CanonicalSlice {
                event: slice.event,
                offset: slice.offset,
                total: slice.total,
                bytes: slice.bytes,
            })
            .collect(),
        next: page.next_offset.map(|offset| CanonicalCursor {
            turn,
            committed: info.committed,
            rolling: info.rolling,
            offset,
        }),
        total_bytes: page.total_bytes,
    })
}
