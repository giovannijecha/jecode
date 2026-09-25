use super::*;
use crate::providers::openai_account::Response;
use crate::providers::openai_account::client::{Attempt, Delivery, RequestStage};
use crate::session::{
    End, Failure, Metrics,
    history::{History, Receipt, Step, Turn},
};

pub(super) fn encode(history: &History) -> Value {
    Value::Array(history.turns.iter().map(encode_turn).collect())
}

pub(super) fn encode_turn(turn: &Turn) -> Value {
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
                        let mut fields = vec![
                            ("text", text(&step.text)),
                            ("reasoning", text(&step.reasoning)),
                            ("accepted", Value::Bool(step.accepted)),
                            (
                                "attempts",
                                Value::Array(step.attempts.iter().map(attempt).collect()),
                            ),
                            (
                                "response",
                                step.response
                                    .as_ref()
                                    .map_or(Value::Null, Response::snapshot),
                            ),
                            (
                                "results",
                                Value::Array(step.results.iter().map(receipt).collect()),
                            ),
                        ];
                        if step.validated_visual_input {
                            fields.push(("validated_visual_input", Value::Bool(true)));
                        }
                        json::object(fields)
                    })
                    .collect(),
            ),
        ),
    ])
}

pub(super) fn decode(value: &Value) -> io::Result<History> {
    let turns = value
        .array()
        .filter(|turns| turns.len() <= 256)
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
                validated_visual_input: match value.get("validated_visual_input") {
                    None => false,
                    Some(Value::Bool(true)) => true,
                    _ => return Err(invalid()),
                },
                response,
                results: Vec::new(),
                attempts: match value.get("attempts") {
                    None => Vec::new(),
                    Some(Value::Array(items)) if items.len() <= 256 => items
                        .iter()
                        .map(read_attempt)
                        .collect::<io::Result<Vec<_>>>()?,
                    _ => return Err(invalid()),
                },
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
                    image: value
                        .get("image")
                        .map(crate::image::Evidence::parse)
                        .transpose()?,
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
                if step.validated_visual_input
                    && (!step.accepted
                        || response.status != crate::providers::openai_account::Status::Completed)
                {
                    return Err(invalid());
                }
            } else if step.accepted || !step.results.is_empty() || step.validated_visual_input {
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

pub(super) fn metrics(m: &Metrics) -> Value {
    let number = |n: Option<u64>| n.map_or(Value::Null, |n| Value::Number(n.to_string()));
    json::object([
        ("requests", number(Some(m.requests.into()))),
        (
            "connection_attempts",
            number(Some(m.connection_attempts.into())),
        ),
        ("submissions", number(Some(m.submissions.into()))),
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

pub(super) fn step_core(step: &Step) -> Value {
    let mut fields = vec![
        ("text", text(&step.text)),
        ("reasoning", text(&step.reasoning)),
        ("accepted", Value::Bool(step.accepted)),
        (
            "attempts",
            Value::Array(step.attempts.iter().map(attempt).collect()),
        ),
        (
            "response",
            step.response
                .as_ref()
                .map_or(Value::Null, Response::snapshot),
        ),
        ("results", Value::Array(Vec::new())),
    ];
    if step.validated_visual_input {
        fields.push(("validated_visual_input", Value::Bool(true)));
    }
    json::object(fields)
}
pub(super) fn receipt(receipt: &Receipt) -> Value {
    let mut fields = vec![
        ("call_id", text(&receipt.call_id)),
        ("output", text(&receipt.output)),
        ("summary", text(&receipt.summary)),
    ];
    if let Some(image) = &receipt.image {
        fields.push(("image", image.value()));
    }
    json::object(fields)
}
pub(super) fn guidance(guidance: &super::super::queue::Guidance) -> Value {
    json::object([
        ("after_step", Value::Number(guidance.after_step.to_string())),
        ("text", text(&guidance.text)),
    ])
}
pub(super) fn end(turn: &Turn) -> Value {
    json::object([
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
        connection_attempts: optional_count(value, "connection_attempts")?,
        submissions: optional_count(value, "submissions")?,
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
fn optional_count(value: &Value, key: &str) -> io::Result<u32> {
    match value.get(key) {
        None => Ok(0),
        Some(value) => value
            .unsigned()
            .and_then(|n| n.try_into().ok())
            .ok_or_else(invalid),
    }
}
pub(super) fn attempt(attempt: &Attempt) -> Value {
    let optional = |value: &Option<String>| value.as_deref().map_or(Value::Null, text);
    json::object([
        (
            "request_sequence",
            Value::Number(attempt.request_sequence.to_string()),
        ),
        (
            "connection_attempt",
            Value::Number(attempt.connection_attempt.to_string()),
        ),
        ("delivery", text(attempt.delivery.name())),
        (
            "stage",
            attempt
                .stage
                .map_or(Value::Null, |stage| text(stage.name())),
        ),
        ("operation", optional(&attempt.operation)),
        (
            "stage_elapsed_ms",
            Value::Number(attempt.stage_elapsed_ms.to_string()),
        ),
        ("category", optional(&attempt.category)),
        (
            "os_code",
            attempt
                .os_code
                .map_or(Value::Null, |code| Value::Number(code.to_string())),
        ),
        (
            "accepted_wire_bytes",
            Value::Number(attempt.accepted_wire_bytes.to_string()),
        ),
        (
            "received_wire_bytes",
            Value::Number(attempt.received_wire_bytes.to_string()),
        ),
        (
            "response_plaintext_bytes",
            Value::Number(attempt.response_plaintext_bytes.to_string()),
        ),
        (
            "response_status",
            attempt
                .response_status
                .map_or(Value::Null, |status| Value::Number(status.to_string())),
        ),
        (
            "stream_events",
            Value::Number(attempt.stream_events.to_string()),
        ),
        ("diagnostic", optional(&attempt.diagnostic)),
        ("retrying", Value::Bool(attempt.retrying)),
    ])
}
pub(super) fn read_attempt(value: &Value) -> io::Result<Attempt> {
    let number = |key| match value.get(key) {
        None => Ok(0),
        Some(value) => value.unsigned().ok_or_else(invalid),
    };
    let optional = |key, max| match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.len() <= max && !text.chars().any(char::is_control) => {
            Ok(Some(text.clone()))
        }
        _ => Err(invalid()),
    };
    let stage = optional("stage", 32)?
        .map(|name| RequestStage::parse(&name).ok_or_else(invalid))
        .transpose()?;
    Ok(Attempt {
        request_sequence: number("request_sequence")?
            .try_into()
            .map_err(|_| invalid())?,
        connection_attempt: number("connection_attempt")?
            .try_into()
            .map_err(|_| invalid())?,
        delivery: Delivery::parse(string(value, "delivery", 32)?).ok_or_else(invalid)?,
        stage,
        stage_elapsed_ms: number("stage_elapsed_ms")?,
        operation: optional("operation", 64)?,
        category: optional("category", 64)?,
        os_code: match value.get("os_code") {
            None | Some(Value::Null) => None,
            Some(Value::Number(n)) => Some(n.parse().map_err(|_| invalid())?),
            _ => return Err(invalid()),
        },
        accepted_wire_bytes: value
            .get("accepted_wire_bytes")
            .and_then(Value::unsigned)
            .and_then(|n| n.try_into().ok())
            .ok_or_else(invalid)?,
        received_wire_bytes: number("received_wire_bytes")?
            .try_into()
            .map_err(|_| invalid())?,
        response_plaintext_bytes: number("response_plaintext_bytes")?
            .try_into()
            .map_err(|_| invalid())?,
        response_status: match value.get("response_status") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .unsigned()
                    .and_then(|n| u16::try_from(n).ok())
                    .filter(|n| (100..=599).contains(n))
                    .ok_or_else(invalid)?,
            ),
        },
        stream_events: number("stream_events")?.try_into().map_err(|_| invalid())?,
        diagnostic: optional("diagnostic", 1024)?,
        retrying: match value.get("retrying") {
            Some(Value::Bool(value)) => *value,
            _ => return Err(invalid()),
        },
    })
}
