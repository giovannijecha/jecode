use super::*;
use crate::providers::openai_account::Response;
use crate::session::{
    End, Failure, Metrics,
    history::{History, Receipt, Step, Turn},
};

pub(super) fn encode(history: &History) -> Value {
    Value::Array(
        history
            .turns
            .iter()
            .map(|turn| {
                json::object([
                    ("prompt", text(&turn.prompt)),
                    ("outcome", text(&turn.outcome)),
                    (
                        "end",
                        text(match turn.end {
                            None => "active",
                            Some(End::Complete) => "complete",
                            Some(End::Incomplete) => "incomplete",
                            Some(End::Refused) => "refused",
                            Some(End::Failed(_)) => "failed",
                        }),
                    ),
                    ("metrics", metrics(&turn.metrics)),
                    (
                        "guidance",
                        Value::Array(
                            turn.guidance
                                .iter()
                                .map(|g| {
                                    json::object([
                                        ("after_step", Value::Number(g.after_step.to_string())),
                                        ("text", text(&g.text)),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                    (
                        "steps",
                        Value::Array(
                            turn.steps
                                .iter()
                                .map(|step| {
                                    json::object([
                                        ("text", text(&step.text)),
                                        ("reasoning", text(&step.reasoning)),
                                        ("accepted", Value::Bool(step.accepted)),
                                        (
                                            "response",
                                            step.response
                                                .as_ref()
                                                .map_or(Value::Null, Response::snapshot),
                                        ),
                                        (
                                            "results",
                                            Value::Array(
                                                step.results
                                                    .iter()
                                                    .map(|receipt| {
                                                        json::object([
                                                            ("call_id", text(&receipt.call_id)),
                                                            ("output", text(&receipt.output)),
                                                            ("summary", text(&receipt.summary)),
                                                        ])
                                                    })
                                                    .collect(),
                                            ),
                                        ),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ])
            })
            .collect(),
    )
}

pub(super) fn decode(value: &Value) -> io::Result<History> {
    let turns = value
        .array()
        .filter(|turns| turns.len() <= super::super::history::MAX_TURNS)
        .ok_or_else(invalid)?;
    let mut history = History::default();
    for (index, value) in turns.iter().enumerate() {
        let end = match string(value, "end", 16)? {
            "active" if index + 1 == turns.len() => None,
            "complete" => Some(End::Complete),
            "incomplete" => Some(End::Incomplete),
            "refused" => Some(End::Refused),
            "failed" => Some(End::Failed(Failure::Worker)),
            _ => return Err(invalid()),
        };
        let mut turn = Turn {
            prompt: string(value, "prompt", 8192)?.into(),
            steps: Vec::new(),
            end,
            outcome: string(value, "outcome", 1024)?.into(),
            metrics: read_metrics(value.get("metrics").ok_or_else(invalid)?)?,
            guidance: Vec::new(),
        };
        for value in value
            .get("steps")
            .and_then(Value::array)
            .filter(|steps| steps.len() <= LIMIT)
            .ok_or_else(invalid)?
        {
            let response = match value.get("response") {
                Some(Value::Null) => None,
                Some(value) => Some(Response::restore(value).map_err(|_| invalid())?),
                None => return Err(invalid()),
            };
            let mut step = Step {
                text: string(value, "text", super::super::history::MAX_TEXT)?.into(),
                reasoning: string(value, "reasoning", super::super::history::MAX_TEXT)?.into(),
                accepted: match value.get("accepted") {
                    Some(Value::Bool(value)) => *value,
                    _ => return Err(invalid()),
                },
                response,
                results: Vec::new(),
            };
            for value in value
                .get("results")
                .and_then(Value::array)
                .filter(|r| r.len() <= 128)
                .ok_or_else(invalid)?
            {
                step.results.push(Receipt {
                    call_id: string(value, "call_id", 256)?.into(),
                    output: string(value, "output", 1024 * 1024)?.into(),
                    summary: string(value, "summary", 8192)?.into(),
                });
            }
            if let Some(response) = &step.response {
                if response.tool_calls.len() != step.results.len()
                    || response
                        .tool_calls
                        .iter()
                        .zip(&step.results)
                        .any(|(call, receipt)| call.id != receipt.call_id)
                    || step.accepted
                        && response.text != step.text
                        && response.legacy_text().map_err(|_| invalid())? != step.text
                {
                    return Err(invalid());
                }
            } else if step.accepted || !step.results.is_empty() {
                return Err(invalid());
            }
            turn.steps.push(step);
        }
        for item in value
            .get("guidance")
            .and_then(Value::array)
            .filter(|items| items.len() <= LIMIT)
            .ok_or_else(invalid)?
        {
            let after_step = item
                .get("after_step")
                .and_then(Value::unsigned)
                .filter(|n| *n <= turn.steps.len() as u64)
                .ok_or_else(invalid)? as usize;
            if turn
                .guidance
                .last()
                .is_some_and(|g| g.after_step > after_step)
            {
                return Err(invalid());
            }
            turn.guidance.push(super::super::queue::Guidance {
                after_step,
                text: string(item, "text", 8192)?.into(),
            });
        }
        history.turns.push(turn);
    }
    Ok(history)
}

fn metrics(m: &Metrics) -> Value {
    let number = |n: Option<u64>| n.map_or(Value::Null, |n| Value::Number(n.to_string()));
    json::object([
        ("requests", number(Some(m.requests.into()))),
        ("tool_calls", number(Some(m.tool_calls.into()))),
        ("elapsed_ms", number(Some(m.elapsed_ms))),
        ("approval_wait_ms", number(Some(m.approval_wait_ms))),
        ("first_text_ms", number(m.first_text_ms)),
        ("input_tokens", number(m.input_tokens)),
        ("output_tokens", number(m.output_tokens)),
        ("cached_tokens", number(m.cached_tokens)),
        ("reasoning_tokens", number(m.reasoning_tokens)),
    ])
}
fn read_metrics(value: &Value) -> io::Result<Metrics> {
    let n = |key| match value.get(key) {
        Some(Value::Null) => Ok(None),
        Some(value) => value.unsigned().map(Some).ok_or_else(invalid),
        None => Err(invalid()),
    };
    Ok(Metrics {
        requests: n("requests")?
            .and_then(|n| n.try_into().ok())
            .ok_or_else(invalid)?,
        tool_calls: n("tool_calls")?
            .and_then(|n| n.try_into().ok())
            .ok_or_else(invalid)?,
        elapsed_ms: n("elapsed_ms")?.ok_or_else(invalid)?,
        approval_wait_ms: n("approval_wait_ms")?.ok_or_else(invalid)?,
        first_text_ms: n("first_text_ms")?,
        input_tokens: n("input_tokens")?,
        output_tokens: n("output_tokens")?,
        cached_tokens: n("cached_tokens")?,
        reasoning_tokens: n("reasoning_tokens")?,
    })
}
