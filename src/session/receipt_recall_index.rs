//! On-demand, bounded addresses for read receipts covered by compaction.
use super::*;
use std::io;

const INDEX_BYTES: usize = 8 * 1024;

fn cursor(turn: usize, step: usize, receipt: usize) -> Value {
    json::object([
        ("mode", string("index")),
        ("turn", number(turn)),
        ("step", number(step)),
        ("receipt", number(receipt)),
    ])
}

fn page(
    start: (usize, usize, usize),
    through: (usize, usize),
    entries: &[Value],
    next: Option<(usize, usize, usize)>,
) -> Value {
    json::object([
        ("ok", Value::Bool(true)),
        ("mode", string("index")),
        (
            "source",
            string("original recorded session calls; no workspace reread"),
        ),
        (
            "identity",
            string("use each returned recall_address for guarded content retrieval"),
        ),
        ("turn", number(start.0)),
        ("step", number(start.1)),
        ("receipt", number(start.2)),
        (
            "covered_through",
            json::object([("turn", number(through.0)), ("step", number(through.1))]),
        ),
        ("entries", Value::Array(entries.to_vec())),
        (
            "next",
            next.map_or(Value::Null, |(t, s, r)| cursor(t, s, r)),
        ),
    ])
}

fn entry(turn: usize, step: usize, receipt: usize, call: &ToolCall, arguments: bool) -> Value {
    json::object([
        ("recall_address", address(turn, step, receipt, 0, &call.id)),
        ("call_name", string(&call.name)),
        (
            "arguments",
            if arguments {
                call.arguments.clone()
            } else {
                Value::Null
            },
        ),
        ("arguments_omitted", Value::Bool(!arguments)),
    ])
}

fn finish(
    start: (usize, usize, usize),
    through: (usize, usize),
    entries: &[Value],
    next: Option<(usize, usize, usize)>,
) -> Output {
    Output::success(
        page(start, through, entries, next),
        format!("indexed {} saved reads", entries.len()),
        next.is_some(),
    )
}

fn append(
    start: (usize, usize, usize),
    through: (usize, usize),
    entries: &mut Vec<Value>,
    position: (usize, usize, usize),
    call: &ToolCall,
) -> Result<(), Output> {
    entries.push(entry(position.0, position.1, position.2, call, true));
    if json::encode(&page(start, through, entries, Some(position)), INDEX_BYTES).is_ok() {
        return Ok(());
    }
    entries.pop();
    if !entries.is_empty() {
        return Err(finish(start, through, entries, Some(position)));
    }
    entries.push(entry(position.0, position.1, position.2, call, false));
    if json::encode(&page(start, through, entries, None), INDEX_BYTES).is_err() {
        return Err(Output::error(
            "recorded call identity exceeds the index limit",
        ));
    }
    Ok(())
}

fn following(
    turn: usize,
    step: usize,
    step_count: usize,
    through: (usize, usize),
) -> Option<(usize, usize, usize)> {
    let next = if step + 1 < step_count {
        (turn, step + 1)
    } else {
        (turn + 1, 0)
    };
    (next < through).then_some((next.0, next.1, 0))
}

/// Lists completed reads already covered by the durable projection. The v2
/// path visits committed events without rebuilding a whole released turn.
pub(crate) fn execute(
    history: &History,
    turn: usize,
    step: usize,
    receipt: usize,
    budget: &Budget<'_>,
) -> Output {
    let start = (turn, step, receipt);
    let (end_turn, end_step, _) = history.projected_cursor();
    let through = (end_turn, end_step);
    if (turn, step) > through || turn > history.turn_count() {
        return Output::error("index cursor is beyond the compacted evidence boundary");
    }
    let mut entries = Vec::new();
    for absolute_turn in turn..=end_turn {
        if budget.check().is_err() {
            return Output::error("receipt index cancelled or timed out");
        }
        if absolute_turn >= history.turn_count() {
            break;
        }
        let resident_index = absolute_turn.checked_sub(history.base_turn);
        let resident = resident_index.and_then(|index| history.turns.get(index));
        let logged =
            resident.is_none() || absolute_turn == history.base_turn && history.base_step > 0;
        let steps = if absolute_turn == history.base_turn {
            history.base_step + resident.map_or(0, |turn| turn.steps.len())
        } else {
            resident.map_or(usize::MAX, |turn| turn.steps.len())
        };
        let last = if absolute_turn == end_turn {
            end_step.min(steps)
        } else {
            steps
        };
        let first_step = if absolute_turn == turn { step } else { 0 };
        for absolute_step in first_step..last {
            if budget.check().is_err() {
                return Output::error("receipt index cancelled or timed out");
            }
            if logged && (resident.is_none() || absolute_step < history.base_step) {
                let Some(record) = history.record.as_ref() else {
                    return Output::error("compacted canonical step is unavailable");
                };
                let first = if absolute_turn == turn && absolute_step == step {
                    receipt
                } else {
                    0
                };
                let saved = match record.indexed_step(absolute_turn, absolute_step, first, budget) {
                    Ok(Some(saved)) => saved,
                    Ok(None) if absolute_turn < end_turn => break,
                    Ok(None) => return Output::error("compacted canonical step is unavailable"),
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                        return Output::error("receipt index cancelled or timed out");
                    }
                    Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                        return Output::error("compacted canonical evidence is corrupt");
                    }
                    Err(_) => return Output::error("compacted canonical step is unavailable"),
                };
                if saved.accepted
                    && let Some(response) = saved
                        .response
                        .as_ref()
                        .filter(|response| response.status == Status::Completed)
                {
                    let end = first
                        .saturating_add(crate::session::persistence::RECEIPT_WINDOW)
                        .min(saved.receipt_base.saturating_add(saved.receipts.len()))
                        .min(response.tool_calls.len());
                    if end <= first && first < response.tool_calls.len() {
                        return Output::error("compacted canonical receipts are incomplete");
                    }
                    for index in first..end {
                        let Some(result) = saved.receipts[index - saved.receipt_base].as_ref()
                        else {
                            return Output::error("compacted canonical receipts are incomplete");
                        };
                        let call = &response.tool_calls[index];
                        if result.call_id != call.id {
                            return Output::error(
                                "compacted canonical receipt identity is corrupt",
                            );
                        }
                        if result.observed
                            && matches!(
                                call.name.as_str(),
                                "list_files" | "read_file" | "search_text"
                            )
                            && let Err(output) = append(
                                start,
                                through,
                                &mut entries,
                                (absolute_turn, absolute_step, index),
                                call,
                            )
                        {
                            return output;
                        }
                    }
                    if end < response.tool_calls.len() {
                        return finish(
                            start,
                            through,
                            &entries,
                            Some((absolute_turn, absolute_step, end)),
                        );
                    }
                }
                return finish(
                    start,
                    through,
                    &entries,
                    following(absolute_turn, absolute_step, saved.step_count, through),
                );
            }
            let saved = if absolute_turn == history.base_turn {
                resident.and_then(|turn| turn.steps.get(absolute_step - history.base_step))
            } else {
                resident.and_then(|turn| turn.steps.get(absolute_step))
            };
            let Some(saved) = saved else {
                return Output::error("compacted canonical step is unavailable");
            };
            let Some(response) = saved
                .response
                .as_ref()
                .filter(|response| saved.accepted && response.status == Status::Completed)
            else {
                continue;
            };
            for (index, (call, result)) in response
                .tool_calls
                .iter()
                .zip(&saved.results)
                .enumerate()
                .skip(if absolute_turn == turn && absolute_step == step {
                    receipt
                } else {
                    0
                })
            {
                if !eligible(call, result) {
                    continue;
                }
                if let Err(output) = append(
                    start,
                    through,
                    &mut entries,
                    (absolute_turn, absolute_step, index),
                    call,
                ) {
                    return output;
                }
            }
        }
    }
    finish(start, through, &entries, None)
}
