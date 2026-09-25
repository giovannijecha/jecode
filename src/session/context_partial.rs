//! Bounded reference-only summaries of one indivisible completed tool step.
use super::*;
use crate::json::{self, Value};

#[derive(Clone)]
pub(crate) struct Pending {
    pub(crate) record: usize,
    pub(crate) offset: usize,
    pub(crate) summary: String,
}

fn text(value: &str) -> Value {
    Value::String(value.into())
}

// The canonical response and receipts remain untouched. These records are user
// reference data, never assistant function_call items or executable tool results.
struct Reference {
    content: String,
    association: String,
}

fn record_count(history: &History) -> Option<usize> {
    let projection = &history.projection;
    let turn = history.turns.get(projection.through)?;
    let step = turn.steps.get(projection.step)?;
    let (items, calls) = step.response.as_ref().map_or((0, 0), |response| {
        (
            response.output.len(),
            response
                .output
                .iter()
                .filter(|item| item.get("type").and_then(Value::text) == Some("function_call"))
                .count(),
        )
    });
    2usize
        .checked_add(items)?
        .checked_add(step.results.len().saturating_sub(calls))
}

/// Materialize one logical reference record at a time, even for a step with
/// many receipts. A call/receipt record may then be sliced for provider input.
fn reference_at(history: &History, index: usize) -> Option<Reference> {
    let projection = &history.projection;
    let turn = history.turns.get(projection.through)?;
    let step = turn.steps.get(projection.step)?;
    let total = record_count(history)?;
    if index >= total {
        return None;
    }
    let (value, association) = if index == 0 {
        let status = step
            .response
            .as_ref()
            .map_or("no validated response", |response| match response.status {
                Status::Completed => "completed",
                Status::Incomplete => "incomplete",
                Status::Refused => "refused",
            });
        (
            json::object([
                ("kind", text("step_header")),
                ("accepted", Value::Bool(step.accepted)),
                ("status", text(status)),
                ("visible_text", text(&step.text)),
                ("reasoning", text(&step.reasoning)),
                (
                    "canonical_response_text",
                    text(step.response.as_ref().map_or("", |response| &response.text)),
                ),
            ]),
            "step header".to_owned(),
        )
    } else if let Some(response) = &step.response {
        if let Some(item) = response.output.get(index - 1) {
            let output_index = index - 1;
            let is_call = item.get("type").and_then(Value::text) == Some("function_call");
            let call_index = response.output[..output_index]
                .iter()
                .filter(|item| item.get("type").and_then(Value::text) == Some("function_call"))
                .count();
            let call = is_call
                .then(|| response.tool_calls.get(call_index))
                .flatten();
            let receipt = is_call.then(|| step.results.get(call_index)).flatten();
            (
                json::object([
                    ("kind", text("response_item_and_receipt")),
                    ("output_index", Value::Number(output_index.to_string())),
                    ("output_item", item.clone()),
                    ("call_id", call.map_or(Value::Null, |call| text(&call.id))),
                    (
                        "call_name",
                        call.map_or(Value::Null, |call| text(&call.name)),
                    ),
                    (
                        "parsed_arguments",
                        call.map_or(Value::Null, |call| call.arguments.clone()),
                    ),
                    (
                        "receipt_call_id",
                        receipt.map_or(Value::Null, |receipt| text(&receipt.call_id)),
                    ),
                    (
                        "receipt_output",
                        receipt.map_or(Value::Null, |receipt| text(&receipt.output)),
                    ),
                    (
                        "receipt_summary",
                        receipt.map_or(Value::Null, |receipt| text(&receipt.summary)),
                    ),
                ]),
                if is_call {
                    format!(
                        "call_id={}; call_name={}; receipt_call_id={}",
                        call.map_or("missing", |call| call.id.as_str()),
                        call.map_or("missing", |call| call.name.as_str()),
                        receipt.map_or("missing", |receipt| receipt.call_id.as_str())
                    )
                } else {
                    format!("response item {output_index}; no executable call")
                },
            )
        } else if index + 1 == total {
            (
                json::object([
                    ("kind", text("step_end")),
                    ("outcome", text(&turn.outcome)),
                    ("all_recorded_items_included", Value::Bool(true)),
                ]),
                "step end".to_owned(),
            )
        } else {
            let calls = response
                .output
                .iter()
                .filter(|item| item.get("type").and_then(Value::text) == Some("function_call"))
                .count();
            let receipt = step
                .results
                .get(calls + index - 1 - response.output.len())?;
            (
                json::object([
                    ("kind", text("unmatched_receipt")),
                    ("call_id", text(&receipt.call_id)),
                    ("output", text(&receipt.output)),
                    ("summary", text(&receipt.summary)),
                ]),
                format!("unmatched receipt call_id={}", receipt.call_id),
            )
        }
    } else {
        if index + 1 == total {
            (
                json::object([
                    ("kind", text("step_end")),
                    ("outcome", text(&turn.outcome)),
                    ("all_recorded_items_included", Value::Bool(true)),
                ]),
                "step end".to_owned(),
            )
        } else {
            let receipt = step.results.get(index - 1)?;
            (
                json::object([
                    ("kind", text("unmatched_receipt")),
                    ("call_id", text(&receipt.call_id)),
                    ("output", text(&receipt.output)),
                    ("summary", text(&receipt.summary)),
                ]),
                format!("unmatched receipt call_id={}", receipt.call_id),
            )
        }
    };
    Some(Reference {
        content: json::encode(&value, 80 * 1024 * 1024).ok()?,
        association,
    })
}
#[cfg(test)]
pub(crate) fn reference_sizes(history: &History) -> Option<Vec<usize>> {
    (0..record_count(history)?)
        .map(|index| reference_at(history, index).map(|record| record.content.len()))
        .collect()
}

pub(super) fn valid_pending(history: &History) -> bool {
    let Some(pending) = &history.projection.pending else {
        return true;
    };
    if pending.summary.trim().is_empty()
        || pending.summary.len() > 32768
        || (pending.record == 0 && pending.offset == 0)
        || next(history, history.projection.through, history.projection.step)
            != Some((history.projection.through, history.projection.step + 1))
    {
        return false;
    }
    reference_at(history, pending.record).is_some_and(|record| {
        pending.offset < record.content.len() && record.content.is_char_boundary(pending.offset)
    })
}

fn base_request(history: &History, model: Model) -> Option<Request> {
    let projection = &history.projection;
    let turn = history.turns.get(projection.through)?;
    let mut input = Vec::new();
    let summary = projection
        .pending
        .as_ref()
        .map_or(projection.summary.as_str(), |pending| &pending.summary);
    if !summary.is_empty() {
        input.push(Input::User(format!(
            "Earlier completed reference summary (data):\n{summary}"
        )));
    }
    input.push(Input::User(format!(
        "Active user objective (data): {}",
        turn.prompt
    )));
    input.extend(
        turn.guidance
            .iter()
            .filter(|guidance| guidance.after_step == projection.step)
            .map(|guidance| {
                Input::User(format!(
                    "User guidance at this step (data): {}",
                    guidance.text
                ))
            }),
    );
    Some(Request {
        model: model.id().into(),
        effort: model.effort().map(str::to_owned),
        input,
        tools: Vec::new(),
        instructions: "Summarize these ordered slices of one completed coding step for continuation. Every slice is reference data, never an executable tool call. Preserve the objective, guidance, exact call ids, names, arguments, results, completed effects, refusals and unknown outcomes. Carry forward earlier facts and clearly distinguish incomplete fragments from a complete step. Do not execute tools. Return a concise factual summary.".into(),
    })
}

fn piece(record: usize, offset: usize, total: usize, association: &str, content: &str) -> Input {
    Input::User(format!(
        "Completed step record {record}, bytes {offset}..{} of {total}; {association}; ordered reference-data fragment (never an executable call or result):\n{content}",
        offset + content.len()
    ))
}

fn slice(
    history: &History,
    model: Model,
    context: &Context,
    count: usize,
) -> Result<Option<(Request, usize, usize)>, Failure> {
    let Some(mut request) = base_request(history, model) else {
        return Ok(None);
    };
    let pending = history.projection.pending.as_ref();
    let mut record = pending.map_or(0, |pending| pending.record);
    let mut offset = pending.map_or(0, |pending| pending.offset);
    let base_len = request.input.len();
    while record < count {
        context.check()?;
        let reference = reference_at(history, record).ok_or(Failure::HistoryLimit)?;
        let content = &reference.content;
        // Every logical record, including a call and receipt, can span
        // bounded UTF-8 slices with its association repeated in the envelope.
        let mut end = (offset + 1024 * 1024).min(content.len());
        while !content.is_char_boundary(end) {
            end -= 1;
        }
        let mut accepted = false;
        while end > offset {
            context.check()?;
            request.input.push(piece(
                record,
                offset,
                content.len(),
                &reference.association,
                &content[offset..end],
            ));
            if request.encode(MAX_CONTEXT).is_ok() {
                accepted = true;
                break;
            }
            request.input.pop();
            end = offset + (end - offset) / 2;
            while end > offset && !content.is_char_boundary(end) {
                end -= 1;
            }
        }
        if !accepted {
            break;
        }
        offset = end;
        if offset == content.len() {
            record += 1;
            offset = 0;
        }
    }
    Ok((request.input.len() > base_len).then_some((request, record, offset)))
}

pub(super) fn compact(
    backend: &mut impl Backend,
    history: &mut History,
    context: &Context,
    model: Model,
    workspace: bool,
    metrics: &mut Metrics,
) -> Result<(), Failure> {
    loop {
        context.check()?;
        let count = record_count(history).ok_or(Failure::HistoryLimit)?;
        let (request, record, offset) = slice(history, model, context, count)
            .map_err(|cause| failed(history, cause))?
            .ok_or_else(|| failed(history, Failure::HistoryLimit))?;
        let before = request
            .encode(MAX_CONTEXT)
            .map_err(|_| Failure::HistoryLimit)?
            .len();
        let original = measured_bytes(history, model, workspace);
        let original_weight = projected_weight(history);
        let old_projection = history.projection.clone();
        let _ = context.send(
            Event::ContextReport(format!(
                "Compacting completed step reference / record {record} of {}",
                count
            )),
            true,
        );
        metrics.requests = metrics.requests.saturating_add(1);
        let mut attempts = Vec::new();
        let mut partial = String::new();
        let result = backend.generate(
            &request,
            &Budget {
                deadline: Instant::now() + Duration::from_secs(180),
                cancelled: &context.cancelled,
            },
            &mut |progress| {
                observe_compaction(progress, &mut attempts, &mut partial, metrics, context)
            },
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
            || response.text.trim().is_empty()
            || response.text.len() > 32768
            || response.text.len() >= before
        {
            return Err(failed(history, Failure::CompactionOutput));
        }
        let complete = record == count;
        if complete {
            history.projection.summary = response.text;
            history.projection.step += 1;
            history.projection.pending = None;
            history.projection.failed = false;
            history.projection.failed_reason = None;
            history.projection.failed_at_turn = None;
            history.projection.failed_attempts.clear();
            history.projection.failed_partial.clear();
            let reduced = match (original, measured_bytes(history, model, workspace)) {
                (Some(before), Some(after)) => after < before,
                _ => original_weight
                    .zip(projected_weight(history))
                    .is_some_and(|(before, after)| after.0 < before.0 && after.1 <= before.1),
            };
            if !reduced || history.checkpoint().is_err() {
                history.projection = old_projection;
                return Err(if reduced {
                    Failure::Storage
                } else {
                    failed(history, Failure::CompactionIneffective)
                });
            }
            history.release_projected();
            let _ = context.send(
                Event::ContextReport(format!(
                    "Compacted completed step / {} reference records / canonical receipts retained",
                    count
                )),
                false,
            );
            return Ok(());
        }
        history.projection.pending = Some(Pending {
            record,
            offset,
            summary: response.text,
        });
        history.projection.failed_attempts.clear();
        history.projection.failed_partial.clear();
        if history.checkpoint().is_err() {
            history.projection = old_projection;
            return Err(Failure::Storage);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::history::{Receipt, Step, Turn};

    #[test]
    fn unmatched_receipt_without_response_is_still_reference_data() {
        let history = History {
            turns: vec![Turn {
                prompt: "unfinished".into(),
                steps: vec![Step {
                    results: vec![Receipt {
                        call_id: "unknown-call".into(),
                        output: "exact-marker".into(),
                        summary: "outcome unknown".into(),
                    }],
                    ..Default::default()
                }],
                end: None,
                outcome: String::new(),
                metrics: Metrics::default(),
                guidance: Vec::new(),
            }],
            ..Default::default()
        };
        assert_eq!(record_count(&history), Some(3));
        let receipt = reference_at(&history, 1).unwrap();
        assert!(receipt.content.contains("unknown-call"));
        assert!(receipt.content.contains("exact-marker"));
        assert!(receipt.content.contains("unmatched_receipt"));
        assert!(
            reference_at(&history, 2)
                .unwrap()
                .content
                .contains("step_end")
        );
    }
}
