//! On-demand, bounded addresses for read receipts covered by compaction.
use super::*;

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

/// Lists only the completed, eligible reads already covered by the durable
/// projection. Released turns are loaded on demand from the committed log.
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
            if resident.is_none() || absolute_turn == history.base_turn && history.base_step > 0 {
                match history
                    .record
                    .as_ref()
                    .and_then(|record| record.recorded_turn(absolute_turn).ok())
                {
                    Some(saved) => Some(saved),
                    None => return Output::error("compacted canonical turn is unavailable"),
                }
            } else {
                None
            };
        let steps = if absolute_turn == history.base_turn {
            history.base_step + resident.map_or(0, |turn| turn.steps.len())
        } else {
            resident
                .or(logged.as_ref())
                .map_or(0, |turn| turn.steps.len())
        };
        let last = if absolute_turn == end_turn {
            end_step.min(steps)
        } else {
            steps
        };
        for absolute_step in if absolute_turn == turn { step } else { 0 }..last {
            if budget.check().is_err() {
                return Output::error("receipt index cancelled or timed out");
            }
            let saved = if absolute_turn == history.base_turn && absolute_step >= history.base_step
            {
                resident.and_then(|turn| turn.steps.get(absolute_step - history.base_step))
            } else if let Some(logged) = &logged {
                logged.steps.get(absolute_step)
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
                let position = (absolute_turn, absolute_step, index);
                entries.push(entry(absolute_turn, absolute_step, index, call, true));
                if json::encode(&page(start, through, &entries, Some(position)), INDEX_BYTES)
                    .is_err()
                {
                    entries.pop();
                    if entries.is_empty() {
                        entries.push(entry(absolute_turn, absolute_step, index, call, false));
                        if json::encode(&page(start, through, &entries, None), INDEX_BYTES).is_err()
                        {
                            return Output::error("recorded call identity exceeds the index limit");
                        }
                        continue;
                    }
                    return Output::success(
                        page(start, through, &entries, Some(position)),
                        format!("indexed {} saved reads", entries.len()),
                        true,
                    );
                }
            }
        }
    }
    Output::success(
        page(start, through, &entries, None),
        format!("indexed {} saved reads", entries.len()),
        false,
    )
}
