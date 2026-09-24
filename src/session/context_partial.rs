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
    atomic: bool,
}

fn records(history: &History) -> Option<Vec<Reference>> {
    let projection = &history.projection;
    let turn = history.turns.get(projection.through)?;
    let step = turn.steps.get(projection.step)?;
    let status = step
        .response
        .as_ref()
        .map_or("no validated response", |response| match response.status {
            Status::Completed => "completed",
            Status::Incomplete => "incomplete",
            Status::Refused => "refused",
        });
    let mut values = vec![(
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
        false,
    )];
    if let Some(response) = &step.response {
        let mut call_index = 0;
        for (output_index, item) in response.output.iter().enumerate() {
            let is_call = item.get("type").and_then(Value::text) == Some("function_call");
            let call = if is_call {
                response.tool_calls.get(call_index)
            } else {
                None
            };
            let receipt = if is_call {
                step.results.get(call_index)
            } else {
                None
            };
            if is_call {
                call_index += 1;
            }
            values.push((
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
                is_call,
            ));
        }
        for receipt in step.results.iter().skip(call_index) {
            values.push((
                json::object([
                    ("kind", text("unmatched_receipt")),
                    ("call_id", text(&receipt.call_id)),
                    ("output", text(&receipt.output)),
                    ("summary", text(&receipt.summary)),
                ]),
                true,
            ));
        }
    }
    values.push((
        json::object([
            ("kind", text("step_end")),
            ("outcome", text(&turn.outcome)),
            ("all_recorded_items_included", Value::Bool(true)),
        ]),
        false,
    ));
    values
        .into_iter()
        .map(|(value, atomic)| {
            Some(Reference {
                content: json::encode(&value, 16 * 1024 * 1024).ok()?,
                atomic,
            })
        })
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
    records(history).is_some_and(|records| {
        records.get(pending.record).is_some_and(|record| {
            pending.offset < record.content.len()
                && record.content.is_char_boundary(pending.offset)
                && (!record.atomic || pending.offset == 0)
        })
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

fn piece(record: usize, offset: usize, total: usize, content: &str) -> Input {
    Input::User(format!(
        "Completed step record {record}, bytes {offset}..{} of {total} (reference data):\n{content}",
        offset + content.len()
    ))
}

fn slice(
    history: &History,
    model: Model,
    context: &Context,
    records: &[Reference],
) -> Result<Option<(Request, usize, usize)>, Failure> {
    let Some(mut request) = base_request(history, model) else {
        return Ok(None);
    };
    let pending = history.projection.pending.as_ref();
    let mut record = pending.map_or(0, |pending| pending.record);
    let mut offset = pending.map_or(0, |pending| pending.offset);
    let base_len = request.input.len();
    while let Some(reference) = records.get(record) {
        context.check()?;
        let content = &reference.content;
        // Keep a call, its exact arguments and its receipt together. Other
        // large reference items can be split at UTF-8 boundaries.
        let mut end = if reference.atomic {
            content.len()
        } else {
            (offset + 1024 * 1024).min(content.len())
        };
        while !content.is_char_boundary(end) {
            end -= 1;
        }
        let mut accepted = false;
        while end > offset {
            context.check()?;
            request
                .input
                .push(piece(record, offset, content.len(), &content[offset..end]));
            if request.encode(MAX_CONTEXT).is_ok() {
                accepted = true;
                break;
            }
            request.input.pop();
            if reference.atomic {
                break;
            }
            end = offset + (end - offset) / 2;
            while end > offset && !content.is_char_boundary(end) {
                end -= 1;
            }
        }
        if !accepted {
            if reference.atomic && request.input.len() == base_len {
                return Ok(None);
            }
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
        let records = records(history).ok_or(Failure::HistoryLimit)?;
        let (request, record, offset) = slice(history, model, context, &records)
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
                records.len()
            )),
            true,
        );
        metrics.requests = metrics.requests.saturating_add(1);
        let result = backend.generate(
            &request,
            &Budget {
                deadline: Instant::now() + Duration::from_secs(180),
                cancelled: &context.cancelled,
            },
            &mut |_| {
                if context.check().is_err() {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        );
        let response = match result {
            Ok(response) => response,
            Err(error) => {
                metrics.usage(&Default::default());
                let cause = failure(error, context);
                return Err(failed(history, cause));
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
        let complete = record == records.len();
        if complete {
            history.projection.summary = response.text;
            history.projection.step += 1;
            history.projection.pending = None;
            history.projection.failed = false;
            history.projection.failed_reason = None;
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
            let _ = context.send(
                Event::ContextReport(format!(
                    "Compacted completed step / {} reference records / canonical receipts retained",
                    records.len()
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
        if history.checkpoint().is_err() {
            history.projection = old_projection;
            return Err(Failure::Storage);
        }
    }
}
