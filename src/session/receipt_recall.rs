//! Bounded, read-only access to this session's committed workspace-read receipts.
use super::history::{History, MAX_TEXT, Receipt, Step};
use crate::{
    json::{self, Value},
    providers::openai_account::{Status, ToolCall},
    tools::{Output, Prepared},
    workspace::Budget,
};

const PAGE_BYTES: usize = 8 * 1024;

/// A page awaiting its first accepted model response is identified from its
/// paired canonical result, not its display summary. Errors and unexecuted
/// calls remain saved but do not pin unrelated reads ahead of compaction.
pub(super) fn admitted(call: &ToolCall, result: &Receipt) -> bool {
    if call.name != "recall_receipts" || call.id != result.call_id || result.output.len() > MAX_TEXT
    {
        return false;
    }
    let Ok(value) = json::parse(
        &result.output,
        json::Limits {
            bytes: MAX_TEXT,
            nodes: 4096,
            depth: 16,
        },
    ) else {
        return false;
    };
    let Ok(Prepared::Recall {
        turn,
        step,
        receipt,
        offset,
    }) = Prepared::parse(&call.name, &call.arguments)
    else {
        return false;
    };
    let coordinate = |key| {
        value
            .get(key)
            .and_then(Value::unsigned)
            .and_then(|n| usize::try_from(n).ok())
    };
    value.get("ok") == Some(&Value::Bool(true))
        && value.get("source").and_then(Value::text).is_some()
        // The inner call_id names the original source read. The outer receipt
        // call_id above pairs this result with the recall call being delivered.
        && value.get("call_id").and_then(Value::text).is_some()
        && matches!(
            value.get("call_name").and_then(Value::text),
            Some("list_files" | "read_file" | "search_text")
        )
        && value.get("output").and_then(Value::text).is_some()
        && (
            coordinate("turn"),
            coordinate("step"),
            coordinate("receipt"),
            coordinate("offset"),
        ) == (Some(turn), Some(step), Some(receipt), Some(offset))
}

fn number(value: usize) -> Value {
    Value::Number(value.to_string())
}
fn string(value: &str) -> Value {
    Value::String(value.into())
}

pub(super) fn execute(
    history: &History,
    turn: usize,
    step: usize,
    receipt: usize,
    offset: usize,
    budget: &Budget<'_>,
) -> Output {
    if budget.check().is_err() {
        return Output::error("receipt recall cancelled or timed out");
    }
    if turn >= history.turn_count() {
        return Output::error("canonical turn does not exist in this session");
    }
    // A released prefix is loaded only through the current session's leased,
    // committed log. Resident indices are never used as canonical coordinates.
    let resident = turn.checked_sub(history.base_turn).and_then(|index| {
        let local_step = if index == 0 {
            step.checked_sub(history.base_step)?
        } else {
            step
        };
        history.turns.get(index)?.steps.get(local_step)
    });
    let loaded = if resident.is_none() {
        match history
            .record
            .as_ref()
            .and_then(|record| record.recorded_turn(turn).ok())
        {
            Some(turn) => Some(turn),
            None => return Output::error("canonical step is unavailable in this session"),
        }
    } else {
        None
    };
    if budget.check().is_err() {
        return Output::error("receipt recall cancelled or timed out");
    }
    let saved_step: Option<&Step> = resident.or_else(|| loaded.as_ref()?.steps.get(step));
    let Some(saved_step) = saved_step else {
        return Output::error("canonical step does not exist in this session");
    };
    let Some(response) = saved_step
        .response
        .as_ref()
        .filter(|response| saved_step.accepted && response.status == Status::Completed)
    else {
        return Output::error("canonical step has no completed, paired receipts");
    };
    let (Some(call), Some(result)) = (
        response.tool_calls.get(receipt),
        saved_step.results.get(receipt),
    ) else {
        return Output::error("receipt index does not exist at this canonical step");
    };
    if !eligible(call, result) {
        return Output::error(
            "this receipt is unexecuted, uncertain or outside the session's workspace-read evidence scope",
        );
    }
    if offset > result.output.len() || !result.output.is_char_boundary(offset) {
        return Output::error(
            "offset must be a valid UTF-8 byte boundary within the recorded result",
        );
    }
    let mut end = (offset + PAGE_BYTES).min(result.output.len());
    while !result.output.is_char_boundary(end) {
        end -= 1;
    }
    loop {
        if budget.check().is_err() {
            return Output::error("receipt recall cancelled or timed out");
        }
        let next = if end < result.output.len() {
            Some((receipt, end))
        } else {
            response
                .tool_calls
                .iter()
                .zip(&saved_step.results)
                .enumerate()
                .skip(receipt + 1)
                .find(|(_, (call, result))| eligible(call, result))
                .map(|(index, _)| (index, 0))
        };
        let next_value = next.map_or(Value::Null, |(receipt, offset)| {
            json::object([
                ("turn", number(turn)),
                ("step", number(step)),
                ("receipt", number(receipt)),
                ("offset", number(offset)),
            ])
        });
        let value = json::object([
            ("ok", Value::Bool(true)),
            (
                "source",
                string("original recorded session receipt; no source reread"),
            ),
            ("turn", number(turn)),
            ("step", number(step)),
            ("receipt", number(receipt)),
            ("receipt_count", number(saved_step.results.len())),
            ("call_id", string(&result.call_id)),
            ("call_name", string(&call.name)),
            ("summary", string(&result.summary)),
            ("offset", number(offset)),
            ("end_offset", number(end)),
            ("total_bytes", number(result.output.len())),
            ("output", string(&result.output[offset..end])),
            ("next", next_value),
        ]);
        if json::encode(&value, crate::tools::MAX_OUTPUT).is_ok() {
            return Output::success(
                value,
                format!(
                    "recorded {} turn {turn} step {step} receipt {receipt}, bytes {offset}..{end}",
                    call.name
                ),
                next.is_some(),
            );
        }
        end = offset + (end - offset) / 2;
        while end > offset && !result.output.is_char_boundary(end) {
            end -= 1;
        }
        if end == offset {
            return Output::error("recorded result metadata exceeds the output limit");
        }
    }
}

fn eligible(
    call: &crate::providers::openai_account::ToolCall,
    result: &super::history::Receipt,
) -> bool {
    if call.id != result.call_id
        || result.summary == "Not executed"
        || !matches!(
            call.name.as_str(),
            "list_files" | "read_file" | "search_text"
        )
        || result.image.is_some()
    {
        return false;
    }
    if result.output.len() <= crate::tools::MAX_OUTPUT
        && let Ok(value) = json::parse(
            &result.output,
            json::Limits {
                bytes: crate::tools::MAX_OUTPUT,
                nodes: 4096,
                depth: 16,
            },
        )
        && (matches!(
            value.get("status").and_then(Value::text),
            Some("not_executed" | "uncertain")
        ) || value.get("executed") == Some(&Value::Bool(false)))
    {
        return false;
    }
    true
}

#[cfg(test)]
#[path = "receipt_recall_cursor_tests.rs"]
mod cursor_tests;
#[cfg(test)]
#[path = "receipt_recall_delivery_tests.rs"]
mod delivery_tests;
#[cfg(test)]
#[path = "receipt_recall_review_tests.rs"]
mod review_tests;
#[cfg(test)]
#[path = "receipt_recall_tests.rs"]
mod tests;
