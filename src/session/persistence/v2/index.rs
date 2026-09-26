//! A bounded view of one step's final canonical response and result identities.
use super::*;
use crate::{
    providers::openai_account::Response,
    session::{history::MAX_TEXT, receipt_recall::observed},
    workspace::Budget,
};
use std::io;

pub(crate) const RECEIPT_WINDOW: usize = 256;
const TURN_CACHE_BYTES: usize = 8 * 1024 * 1024;

pub(crate) struct IndexedReceipt {
    pub call_id: String,
    pub observed: bool,
}

pub(crate) struct IndexedStep {
    pub response: Option<Response>,
    pub accepted: bool,
    pub receipts: Vec<Option<IndexedReceipt>>,
    pub receipt_base: usize,
    pub step_count: usize,
}

pub(super) struct Cache {
    turn: usize,
    step: usize,
    committed: u64,
    rolling: u64,
    file_len: u64,
    modified: std::time::SystemTime,
    saved: Arc<IndexedStep>,
}

/// Only call metadata and result identity are retained; source outputs stay in
/// canonical frames. None means this turn exceeded the bounded cache budget.
pub(super) struct TurnCache {
    turn: usize,
    committed: u64,
    rolling: u64,
    file_len: u64,
    modified: std::time::SystemTime,
    steps: Option<Vec<Arc<IndexedStep>>>,
}

fn corrupt() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "committed receipt events are corrupt",
    )
}

fn coordinate(event: &Value, key: &str) -> io::Result<usize> {
    event
        .get(key)
        .and_then(Value::unsigned)
        .and_then(|value| value.try_into().ok())
        .ok_or_else(corrupt)
}

fn result_identity(data: &Value) -> io::Result<IndexedReceipt> {
    let call_id = data
        .get("call_id")
        .and_then(Value::text)
        .filter(|id| id.len() <= 256)
        .ok_or_else(corrupt)?;
    let summary = data
        .get("summary")
        .and_then(Value::text)
        .filter(|summary| summary.len() <= 8192)
        .ok_or_else(corrupt)?;
    let output = data
        .get("output")
        .and_then(Value::text)
        .filter(|output| output.len() <= MAX_TEXT)
        .ok_or_else(corrupt)?;
    Ok(IndexedReceipt {
        call_id: call_id.into(),
        observed: observed(summary, output, data.get("image").is_some()),
    })
}

/// One verified scan may serve many index pages. If metadata would exceed the
/// fixed budget, the caller falls back to bounded step windows instead.
fn build_turn_cache(
    record: &Record,
    turn: usize,
    budget: &Budget<'_>,
) -> io::Result<Option<Vec<Arc<IndexedStep>>>> {
    let mut begun = false;
    let mut count = 0usize;
    let mut bytes = 0usize;
    let mut oversized = false;
    let mut steps: Vec<IndexedStep> = Vec::new();
    visit_events(
        record,
        turn,
        || {
            budget
                .check()
                .map_err(|_| io::Error::from(io::ErrorKind::Interrupted))
        },
        |event| {
            let kind = event
                .get("kind")
                .and_then(Value::text)
                .ok_or_else(corrupt)?;
            let position = coordinate(&event, "step")?;
            let index = coordinate(&event, "index")?;
            let data = event.get("data").ok_or_else(corrupt)?;
            match kind {
                "begin" if !begun && position == 0 && index == 0 => begun = true,
                "step" if begun && index == 0 && position <= count => {
                    if position == count {
                        count += 1;
                    }
                    if oversized {
                        return Ok(());
                    }
                    let size = json::encode(data, TURN_CACHE_BYTES)
                        .map_or(TURN_CACHE_BYTES + 1, |value| value.len());
                    bytes = bytes.saturating_add(size);
                    if bytes > TURN_CACHE_BYTES {
                        oversized = true;
                        steps.clear();
                        return Ok(());
                    }
                    let accepted = match data.get("accepted") {
                        Some(Value::Bool(value)) => *value,
                        _ => return Err(corrupt()),
                    };
                    let response = match data.get("response") {
                        Some(Value::Null) => None,
                        Some(value) => Some(Response::restore(value).map_err(|_| corrupt())?),
                        None => return Err(corrupt()),
                    };
                    let calls = response
                        .as_ref()
                        .map_or(0, |response| response.tool_calls.len());
                    if position == steps.len() {
                        steps.push(IndexedStep {
                            response,
                            accepted,
                            receipts: (0..calls).map(|_| None).collect(),
                            receipt_base: 0,
                            step_count: 0,
                        });
                    } else {
                        let saved = &mut steps[position];
                        saved.response = response;
                        saved.accepted = accepted;
                        saved.receipts.resize_with(calls, || None);
                    }
                }
                "receipt" if begun && position < count => {
                    if oversized {
                        return Ok(());
                    }
                    let saved = steps.get_mut(position).ok_or_else(corrupt)?;
                    let slot = saved.receipts.get_mut(index).ok_or_else(corrupt)?;
                    let identity = result_identity(data)?;
                    bytes = bytes.saturating_add(identity.call_id.len() + 64);
                    if bytes > TURN_CACHE_BYTES {
                        oversized = true;
                        steps.clear();
                        return Ok(());
                    }
                    *slot = Some(identity);
                }
                "guidance" | "end" if begun => {}
                _ => return Err(corrupt()),
            }
            Ok(())
        },
    )?;
    if !begun {
        return Err(corrupt());
    }
    if oversized {
        return Ok(None);
    }
    if count != steps.len() {
        return Err(corrupt());
    }
    for saved in &mut steps {
        saved.step_count = count;
    }
    Ok(Some(steps.into_iter().map(Arc::new).collect()))
}

/// Later checkpoint updates replace earlier events. The retained window never
/// includes source output bytes, and a result frame is parsed only transiently.
pub(crate) fn recorded_step(
    record: &crate::session::persistence::Record,
    turn: usize,
    step: usize,
    receipt: usize,
    budget: &Budget<'_>,
) -> io::Result<Option<Arc<IndexedStep>>> {
    budget
        .check()
        .map_err(|_| io::Error::from(io::ErrorKind::Interrupted))?;
    let head = checked_head(&record.store, &record.id)?;
    let metadata = record
        .store
        .read_file(&format!("{}.log", record.id))?
        .metadata()?;
    let modified = metadata.modified()?;
    let mut oversized_turn = false;
    if let Some(tracker) = &record.incremental {
        let guard = tracker
            .lock()
            .map_err(|_| io::Error::other("session log lock poisoned"))?;
        if let Some(cache) = &guard.turn_index_cache
            && cache.turn == turn
            && cache.committed == head.committed
            && cache.rolling == head.rolling
            && cache.file_len == metadata.len()
            && cache.modified == modified
        {
            if let Some(steps) = &cache.steps {
                return Ok(steps.get(step).cloned());
            }
            oversized_turn = true;
        }
        if let Some(cache) = &guard.index_cache
            && cache.turn == turn
            && cache.step == step
            && cache.committed == head.committed
            && cache.rolling == head.rolling
            && cache.file_len == metadata.len()
            && cache.modified == modified
            && (cache.saved.receipt_base..cache.saved.receipt_base.saturating_add(RECEIPT_WINDOW))
                .contains(&receipt)
        {
            return Ok(Some(Arc::clone(&cache.saved)));
        }
    }
    if !oversized_turn {
        let steps = build_turn_cache(record, turn, budget)?;
        let after = checked_head(&record.store, &record.id)?;
        let file_after = record
            .store
            .read_file(&format!("{}.log", record.id))?
            .metadata()?;
        if after.committed != head.committed
            || after.rolling != head.rolling
            || file_after.len() != metadata.len()
            || file_after.modified()? != modified
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "canonical log changed during receipt traversal",
            ));
        }
        let complete = steps.is_some();
        let found = steps.as_ref().and_then(|steps| steps.get(step).cloned());
        if let Some(tracker) = &record.incremental {
            tracker
                .lock()
                .map_err(|_| io::Error::other("session log lock poisoned"))?
                .turn_index_cache = Some(Arc::new(TurnCache {
                turn,
                committed: head.committed,
                rolling: head.rolling,
                file_len: metadata.len(),
                modified,
                steps,
            }));
        }
        if complete {
            return Ok(found);
        }
    }
    let mut begun = false;
    let mut step_count = 0usize;
    let mut core = None;
    let mut results: Vec<Option<IndexedReceipt>> = (0..RECEIPT_WINDOW).map(|_| None).collect();
    let end = receipt.saturating_add(RECEIPT_WINDOW);
    record.visit_canonical_events(
        turn,
        || {
            budget
                .check()
                .map_err(|_| io::Error::from(io::ErrorKind::Interrupted))
        },
        |event| {
            let kind = event
                .get("kind")
                .and_then(Value::text)
                .ok_or_else(corrupt)?;
            let position = coordinate(&event, "step")?;
            let index = coordinate(&event, "index")?;
            let data = event.get("data").ok_or_else(corrupt)?;
            match kind {
                "begin" if !begun && position == 0 && index == 0 => begun = true,
                "step" if begun && index == 0 && position <= step_count => {
                    if position == step_count {
                        step_count += 1;
                    }
                    if position == step {
                        let accepted = match data.get("accepted") {
                            Some(Value::Bool(value)) => *value,
                            _ => return Err(corrupt()),
                        };
                        let response = match data.get("response") {
                            Some(Value::Null) => None,
                            Some(value) => Some(Response::restore(value).map_err(|_| corrupt())?),
                            None => return Err(corrupt()),
                        };
                        core = Some((accepted, response));
                    }
                }
                "receipt" if begun && position < step_count => {
                    if position == step && (receipt..end).contains(&index) {
                        results[index - receipt] = Some(result_identity(data)?);
                    }
                }
                "guidance" | "end" if begun => {}
                _ => return Err(corrupt()),
            }
            Ok(())
        },
    )?;
    if !begun {
        return Err(corrupt());
    }
    if step >= step_count {
        return Ok(None);
    }
    let (accepted, response) = core.ok_or_else(corrupt)?;
    let saved = Arc::new(IndexedStep {
        response,
        accepted,
        receipts: results,
        receipt_base: receipt,
        step_count,
    });
    let after = checked_head(&record.store, &record.id)?;
    let file_after = record
        .store
        .read_file(&format!("{}.log", record.id))?
        .metadata()?;
    if after.committed != head.committed
        || after.rolling != head.rolling
        || file_after.len() != metadata.len()
        || file_after.modified()? != modified
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "canonical log changed during receipt traversal",
        ));
    }
    if let Some(tracker) = &record.incremental {
        tracker
            .lock()
            .map_err(|_| io::Error::other("session log lock poisoned"))?
            .index_cache = Some(Arc::new(Cache {
            turn,
            step,
            committed: head.committed,
            rolling: head.rolling,
            file_len: metadata.len(),
            modified,
            saved: Arc::clone(&saved),
        }));
    }
    Ok(Some(saved))
}
