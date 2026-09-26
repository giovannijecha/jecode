//! A checkpointed model-input cursor over immutable canonical turns and receipts.
use super::{
    Event, Failure, Metrics, Model,
    history::{History, MAX_CONTEXT, MAX_REQUEST},
    worker::{Backend, Context, failure},
};
use crate::{
    providers::openai_account::{Input, Progress, Request, Status, client::Attempt},
    tls::Budget,
};
use std::ops::ControlFlow;

#[derive(Clone)]
pub(super) struct AbandonedVisual {
    pub from: (usize, usize),
    pub through: (usize, usize),
}

#[derive(Clone)]
pub(super) struct Projection {
    pub through: usize,
    pub step: usize,
    pub summary: String,
    /// Bounded source user messages, separate from model-generated prose.
    pub source: Vec<handoff::SourceUser>,
    pub source_omitted: usize,
    pub limit_bytes: usize,
    pub failed: bool,
    pub failed_reason: Option<Failure>,
    /// Absolute canonical turn count when the latest failed compaction ran.
    pub failed_at_turn: Option<usize>,
    pub pending: Option<partial::Pending>,
    pub failed_attempts: Vec<Attempt>,
    pub failed_partial: String,
    /// Absolute canonical step ranges explicitly removed from visual projection.
    /// Receipts and private image files are still retained.
    pub abandoned_visual: Vec<AbandonedVisual>,
}
impl Default for Projection {
    fn default() -> Self {
        Self {
            through: 0,
            step: 0,
            summary: String::new(),
            source: Vec::new(),
            source_omitted: 0,
            limit_bytes: 512 * 1024,
            failed: false,
            failed_reason: None,
            failed_at_turn: None,
            pending: None,
            failed_attempts: Vec::new(),
            failed_partial: String::new(),
            abandoned_visual: Vec::new(),
        }
    }
}
#[cfg(test)]
#[path = "context_contract_tests.rs"]
mod contract_tests;
#[path = "context_handoff.rs"]
pub(super) mod handoff;
#[path = "context_partial.rs"]
pub(super) mod partial;
#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;

fn request_bytes(
    history: &History,
    model: Model,
    workspace: bool,
) -> Result<Option<usize>, Failure> {
    Ok(history
        .projected_request(model, workspace)?
        .encode(MAX_REQUEST)
        .ok()
        .map(|s| s.len()))
}
fn measured_bytes(history: &History, model: Model, workspace: bool) -> Option<usize> {
    history
        .projected_request(model, workspace)
        .ok()?
        .encode(80 * 1024 * 1024)
        .ok()
        .map(|s| s.len())
}
fn projected_weight(history: &History) -> Option<(usize, usize)> {
    let input = history.input(history.turns.len()).ok()?;
    let mut bytes = 0usize;
    let mut items = 0usize;
    for item in input {
        match item {
            Input::User(text) => {
                bytes = bytes.checked_add(text.len())?;
                items += 1;
            }
            Input::ToolResult { call_id, output } => {
                bytes = bytes
                    .checked_add(call_id.len())?
                    .checked_add(output.len())?;
                items += 1;
            }
            Input::ToolImage {
                call_id,
                description,
                image_url,
            } => {
                bytes = bytes
                    .checked_add(call_id.len())?
                    .checked_add(description.len())?
                    .checked_add(image_url.len())?;
                items += 1;
            }
            Input::Assistant(output) => {
                items = items.checked_add(output.len())?;
                bytes = bytes.checked_add(
                    crate::json::encode(&crate::json::Value::Array(output), 80 * 1024 * 1024)
                        .ok()?
                        .len(),
                )?;
            }
        }
    }
    Some((bytes, items))
}
pub(super) fn report(history: &History, model: Model, workspace: bool) -> String {
    let bytes = request_bytes(history, model, workspace).ok().flatten();
    let mut message = format!(
        "Context / {} canonical turns / {} turns and {} steps summarized\nRequest JSON: {} bytes / compaction threshold: {} bytes",
        history.turn_count(),
        history.base_turn + history.projection.through,
        if history.projection.through == 0 {
            history.base_step + history.projection.step
        } else {
            history.projection.step
        },
        bytes.map_or("over limit".into(), |n| n.to_string()),
        history.projection.limit_bytes
    );
    if let Some(usage) = history
        .turns
        .iter()
        .rev()
        .flat_map(|t| t.steps.iter().rev())
        .find_map(|s| s.response.as_ref().map(|r| &r.usage))
    {
        let count = |n: Option<u64>| n.map_or("unknown".into(), |n| n.to_string());
        message.push_str(&format!(
            "\nLast completed conversation response: {} input tokens / {} cached / {} output",
            count(usage.input),
            count(usage.cached),
            count(usage.output)
        ));
    }
    message.push_str("\nBytes are measured locally; token counts are reported by the provider. Canonical history is retained.");
    if history.projection.failed {
        if let Some(last) = history.projection.failed_attempts.last() {
            message.push_str(&format!(
                "\nLast compaction attempt: {} / {}",
                last.delivery,
                last.diagnostic
                    .as_deref()
                    .unwrap_or("no validated completion")
            ));
        }
        if !history.projection.failed_partial.is_empty() {
            let excerpt = history
                .projection
                .failed_partial
                .chars()
                .take(4096)
                .collect::<String>();
            message.push_str(&format!(
                "\nUnvalidated partial summary (reference only): {excerpt}"
            ));
        }
    }
    if history.pending_image() {
        message.push_str(if history.can_view_images() {
            "\nPending image evidence: saved pixels remain in the next visual request until a validated response completes."
        } else {
            "\nPending image evidence: the selected model receives text references only; saved pixels remain available for an image-capable model."
        });
    }
    if history.pending_image() && history.can_view_images() && bytes.is_none() {
        message.push_str("\nThis saved visual request exceeds 8 MiB. Run /discard-pending-images to stop sending the current pending pixels without claiming inspection, then request smaller PNG views.");
    }
    message
}

pub(super) fn observe_compaction(
    progress: Progress<'_>,
    attempts: &mut Vec<Attempt>,
    partial: &mut String,
    metrics: &mut Metrics,
    context: &Context,
) -> ControlFlow<()> {
    match progress {
        Progress::Attempt(mut attempt) => {
            attempt.request_sequence = metrics.requests;
            metrics.observe(&attempt);
            if attempt.retrying {
                let _ = context.send(Event::Retrying, true);
            }
            attempts.push(attempt);
            ControlFlow::Continue(())
        }
        Progress::Text(text) | Progress::Reasoning(text) => {
            if text.len() <= 32768usize.saturating_sub(partial.len()) {
                partial.push_str(text);
            }
            if context.check().is_err() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }
    }
}

pub(super) fn failed_attempt(
    history: &mut History,
    cause: Failure,
    attempts: Vec<Attempt>,
    partial: String,
) -> Failure {
    history.projection.failed_attempts = attempts;
    history.projection.failed_partial = partial;
    history.projection.failed_at_turn = Some(history.turn_count());
    commit_failure(history, cause)
}

pub(super) fn ensure(
    backend: &mut impl Backend,
    history: &mut History,
    context: &Context,
    model: Model,
    workspace: bool,
    metrics: &mut Metrics,
) -> Result<(), Failure> {
    loop {
        let size = request_bytes(history, model, workspace)?;
        // A captured image remains pending across failed requests, ended turns
        // and resume. Send its pixels before applying the text threshold.
        let visual_pending = history.pending_image() && history.can_view_images();
        let recalled_pending = history.pending_recall_step().is_some();
        if (visual_pending || recalled_pending)
            && history.projection.pending.is_none()
            && size.is_some()
        {
            return Ok(());
        }
        let limit = if visual_pending {
            Failure::ImageRequestLimit
        } else {
            Failure::HistoryLimit
        };
        let bounded_text_size = size.filter(|bytes| *bytes <= MAX_CONTEXT);
        if history.projection.pending.is_none()
            && size.is_some_and(|n| n <= history.projection.limit_bytes)
        {
            return Ok(());
        }
        if history.projection.failed {
            return bounded_text_size.map_or_else(
                || {
                    Err(history
                        .projection
                        .failed_reason
                        .unwrap_or(Failure::HistoryLimit))
                },
                |_| Ok(()),
            );
        }
        let candidate = next_eligible(
            history,
            history.projection.through,
            history.projection.step,
            history
                .pending_image_step()
                .into_iter()
                .chain(history.pending_recall_step())
                .min(),
        );
        if history.projection.pending.is_none() && candidate.is_none() {
            return bounded_text_size.map_or(Err(limit), |_| Ok(()));
        }
        let before = (
            history.projection.through,
            history.projection.step,
            history.projection.summary.clone(),
            history.projection.pending.is_some(),
        );
        compact(backend, history, context, model, workspace, metrics)?;
        if before
            == (
                history.projection.through,
                history.projection.step,
                history.projection.summary.clone(),
                history.projection.pending.is_some(),
            )
        {
            return bounded_text_size.map_or(Err(limit), |_| Ok(()));
        }
    }
}

// Only a completed paired step, or a fully ended turn, can advance the cursor.
fn next(history: &History, through: usize, step: usize) -> Option<(usize, usize)> {
    let turn = history.turns.get(through)?;
    if let Some(item) = turn.steps.get(step) {
        if turn.end.is_some() {
            return Some((through, step + 1));
        }
        let response = item.response.as_ref()?;
        if !item.accepted
            || response.status == Status::Incomplete
            || response.tool_calls.len() != item.results.len()
            || response
                .tool_calls
                .iter()
                .zip(&item.results)
                .any(|(call, receipt)| call.id != receipt.call_id)
            || item
                .results
                .iter()
                .any(|receipt| receipt.summary == "Not executed")
        {
            return None;
        }
        return Some((through, step + 1));
    }
    turn.end.map(|_| (through + 1, 0))
}

fn next_eligible(
    history: &History,
    through: usize,
    step: usize,
    protected: Option<(usize, usize)>,
) -> Option<(usize, usize)> {
    let current = (through, step);
    if protected.is_some_and(|position| current >= position) {
        return None;
    }
    next(history, through, step)
        .filter(|candidate| protected.is_none_or(|position| *candidate <= position))
}

pub(super) fn eligible_before_current_step(history: &History) -> bool {
    let Some(turn) = history.turns.last() else {
        return false;
    };
    let Some(step) = turn.steps.len().checked_sub(1) else {
        return false;
    };
    next_eligible(
        history,
        history.projection.through,
        history.projection.step,
        Some((history.turns.len() - 1, step)),
    )
    .is_some()
}

pub(super) fn valid_cursor(history: &History) -> bool {
    let target = (history.projection.through, history.projection.step);
    let mut cursor = (0, 0);
    while cursor < target {
        let Some(candidate) = next(history, cursor.0, cursor.1) else {
            return false;
        };
        cursor = candidate;
    }
    cursor == target && partial::valid_pending(history)
}

fn segment(history: &History, to: (usize, usize)) -> Result<Vec<Input>, Failure> {
    let mut input = Vec::new();
    if !history.projection.summary.is_empty() {
        input.push(Input::User(format!(
            "Earlier validated handoff (reference data):\n{}",
            history.projection.summary
        )));
    }
    if let Some(source) = handoff::source_reference(
        &history.projection.source,
        history.projection.source_omitted,
    ) {
        input.push(Input::User(source));
    }
    for (turn_index, turn) in history
        .turns
        .iter()
        .enumerate()
        .take(to.0 + usize::from(to.1 > 0))
        .skip(history.projection.through)
    {
        let first = if turn_index == history.projection.through {
            history.projection.step
        } else {
            0
        };
        let last = if turn_index == to.0 {
            to.1
        } else {
            turn.steps.len()
        };
        if first == 0 || turn_index == history.projection.through {
            input.push(Input::User(format!(
                "Canonical user request, turn {} (reference data): {:?}",
                history.base_turn + turn_index,
                turn.prompt
            )));
        }
        for index in first..last {
            let absolute_step = if turn_index == 0 {
                history.base_step + index
            } else {
                index
            };
            for guidance in turn.guidance.iter().filter(|g| g.after_step == index) {
                input.push(Input::User(format!(
                    "Canonical user guidance, turn {} before step {} (reference data): {:?}",
                    history.base_turn + turn_index,
                    absolute_step,
                    guidance.text
                )));
            }
            let count = partial::record_count_at(history, turn_index, index)
                .ok_or(Failure::HistoryLimit)?;
            for record in 0..count {
                let reference = partial::reference_at_position(history, turn_index, index, record)
                    .ok_or(Failure::HistoryLimit)?;
                input.push(Input::User(format!(
                    "Canonical turn {} step {} record {} of {} / {} / ordered non-executing reference data:\n{}",
                    history.base_turn + turn_index, absolute_step, record + 1, count,
                    reference.association, reference.content
                )));
            }
        }
        if turn_index < to.0 {
            for guidance in turn
                .guidance
                .iter()
                .filter(|g| g.after_step == turn.steps.len())
            {
                input.push(Input::User(format!(
                    "Canonical user guidance, turn {} after step {} (reference data): {:?}",
                    history.base_turn + turn_index,
                    if turn_index == 0 {
                        history.base_step + turn.steps.len()
                    } else {
                        turn.steps.len()
                    },
                    guidance.text
                )));
            }
            input.push(Input::User(format!(
                "Canonical turn {} outcome (reference data): {:?}",
                history.base_turn + turn_index,
                turn.outcome
            )));
        }
    }
    Ok(input)
}

fn summary_request(
    history: &History,
    model: Model,
    to: (usize, usize),
) -> Result<Request, Failure> {
    Ok(Request {
        model: model.id().into(),
        effort: model.effort().map(str::to_owned),
        input: segment(history, to)?,
        tools: Vec::new(),
        instructions: handoff::instructions(&handoff::boundary(history, to)),
    })
}

fn failed(history: &mut History, cause: Failure) -> Failure {
    history.projection.failed_at_turn = None;
    commit_failure(history, cause)
}

fn commit_failure(history: &mut History, cause: Failure) -> Failure {
    history.projection.failed = true;
    history.projection.failed_reason = Some(cause);
    if history.checkpoint().is_err() {
        Failure::Storage
    } else {
        cause
    }
}

pub(super) fn compact(
    backend: &mut impl Backend,
    history: &mut History,
    context: &Context,
    model: Model,
    workspace: bool,
    metrics: &mut Metrics,
) -> Result<(), Failure> {
    context.check()?;
    let start = (history.projection.through, history.projection.step);
    let protected = history
        .pending_image_step()
        .into_iter()
        .chain(history.pending_recall_step())
        .min();
    if protected.is_some_and(|position| start >= position) {
        if request_bytes(history, model, workspace)?.is_none() {
            return Err(
                if history.pending_image_step() == protected && history.can_view_images() {
                    Failure::ImageRequestLimit
                } else {
                    Failure::HistoryLimit
                },
            );
        }
        let message = if history.pending_image_step() == protected {
            "Pending image pixels are retained until a validated visual response; no earlier context is eligible for compaction"
        } else {
            "Recalled canonical receipts are retained until a following validated response; no earlier context is eligible for compaction"
        };
        let _ = context.send(Event::ContextReport(message.into()), false);
        return Ok(());
    }
    if history.projection.pending.is_some() {
        return partial::compact(backend, history, context, model, workspace, metrics);
    }
    // The encoder's 2 MiB wire bound is not a claimed model token capacity.
    let budget = MAX_CONTEXT;
    let mut cursor = start;
    while let Some(candidate) = next_eligible(history, cursor.0, cursor.1, protected) {
        if summary_request(history, model, candidate)?
            .encode(budget)
            .is_err()
        {
            break;
        }
        cursor = candidate;
    }
    if cursor == start {
        if history
            .turns
            .get(start.0)
            .and_then(|turn| turn.steps.get(start.1))
            .is_some()
            && next_eligible(history, start.0, start.1, protected) == Some((start.0, start.1 + 1))
        {
            return partial::compact(backend, history, context, model, workspace, metrics);
        }
        if request_bytes(history, model, workspace)?.is_some() {
            let _ = context.send(
                Event::ContextReport("No completed context fits a bounded summary request".into()),
                false,
            );
            return Ok(());
        }
        let limit = if history.pending_image() && history.can_view_images() {
            Failure::ImageRequestLimit
        } else {
            Failure::HistoryLimit
        };
        return Err(failed(history, limit));
    }
    let request = summary_request(history, model, cursor)?;
    let before = request
        .encode(budget)
        .map_err(|_| Failure::HistoryLimit)?
        .len();
    let original = measured_bytes(history, model, workspace);
    let original_weight = projected_weight(history);
    let _ = context.send(
        Event::ContextReport(
            "Compacting completed context / canonical history stays intact".into(),
        ),
        true,
    );
    metrics.requests = metrics.requests.saturating_add(1);
    let mut attempts = Vec::new();
    let mut partial = String::new();
    let result = backend.generate(
        &request,
        &Budget {
            deadline: None,
            cancelled: &context.cancelled,
        },
        &mut |progress| observe_compaction(progress, &mut attempts, &mut partial, metrics, context),
    );
    let response = match result {
        Ok(response) => response,
        Err(error) => {
            metrics.usage(&Default::default());
            let cause = failure(error, context);
            return Err(failed_attempt(history, cause, attempts, partial));
        }
    };
    metrics.usage(&response.usage);
    context.check().map_err(|cause| failed(history, cause))?;
    if response.status != Status::Completed
        || !response.tool_calls.is_empty()
        || handoff::valid(&response.text, &handoff::boundary(history, cursor)).is_err()
        || response.text.len() >= before
    {
        return Err(failed(history, Failure::CompactionOutput));
    }
    let limit_bytes = history.projection.limit_bytes;
    let abandoned_visual = history.projection.abandoned_visual.clone();
    let (source, source_omitted) = handoff::with_covered(history, start, cursor);
    let old = std::mem::replace(
        &mut history.projection,
        Projection {
            through: cursor.0,
            step: cursor.1,
            summary: response.text,
            source,
            source_omitted,
            limit_bytes,
            failed: false,
            failed_reason: None,
            failed_at_turn: None,
            pending: None,
            failed_attempts: Vec::new(),
            failed_partial: String::new(),
            abandoned_visual,
        },
    );
    let reduced = match (original, measured_bytes(history, model, workspace)) {
        (Some(before), Some(after)) => after < before,
        _ => original_weight
            .zip(projected_weight(history))
            .is_some_and(|(before, after)| after.0 < before.0 && after.1 <= before.1),
    };
    if !reduced {
        history.projection = old;
        return Err(failed(history, Failure::CompactionIneffective));
    }
    if history.checkpoint().is_err() {
        history.projection = old;
        return Err(Failure::Storage);
    }
    history.release_projected();
    let _ = context.send(
        Event::ContextReport(format!(
            "Compacted through turn {} step {} / summary {} bytes / canonical history retained",
            cursor.0,
            cursor.1,
            history.projection.summary.len()
        )),
        false,
    );
    Ok(())
}
