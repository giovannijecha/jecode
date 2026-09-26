//! Rebuild only the unprojected suffix; older frames are checksum validated.
use super::{codec, log};
use crate::{json::Value, session::history::History};
use std::io;

pub(super) struct Replay {
    first: usize,
    first_step: usize,
    first_guidance: usize,
    remaining: Option<usize>,
    turns: Vec<Value>,
}
impl Replay {
    pub(super) fn new(first: usize, first_step: usize, first_guidance: usize) -> Self {
        Self {
            first,
            first_step,
            first_guidance,
            remaining: None,
            turns: Vec::new(),
        }
    }
    pub(super) fn limited(
        first: usize,
        first_step: usize,
        first_guidance: usize,
        bytes: usize,
    ) -> Self {
        Self {
            first,
            first_step,
            first_guidance,
            remaining: Some(bytes),
            turns: Vec::new(),
        }
    }
    pub(super) fn apply(&mut self, turn: usize, event: Value) -> io::Result<()> {
        if let Some(remaining) = &mut self.remaining {
            let size = crate::json::encode(&event, super::log::EVENT_LIMIT)
                .map_err(|_| log::corrupt())?
                .len();
            *remaining = remaining.checked_sub(size).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "canonical page exceeds its byte budget",
                )
            })?;
        }
        let kind = event
            .get("kind")
            .and_then(Value::text)
            .ok_or_else(log::corrupt)?
            .to_owned();
        let step = field(&event, "step")?;
        let index = field(&event, "index")?;
        let data = event.get("data").cloned().ok_or_else(log::corrupt)?;
        if kind == "begin" {
            if turn != self.first + self.turns.len() || step != 0 || index != 0 {
                return Err(log::corrupt());
            }
            self.turns.push(data);
            return Ok(());
        }
        let value = self
            .turns
            .get_mut(turn.checked_sub(self.first).ok_or_else(log::corrupt)?)
            .ok_or_else(log::corrupt)?;
        let step_base = if turn == self.first {
            self.first_step
        } else {
            0
        };
        match kind.as_str() {
            "step" => {
                if step < step_base {
                    return Ok(());
                }
                let step = step - step_base;
                let steps = array(value, "steps")?;
                if step > steps.len() || index != 0 {
                    return Err(log::corrupt());
                }
                let mut data = data;
                if step < steps.len() {
                    let previous = steps[step]
                        .get("results")
                        .cloned()
                        .ok_or_else(log::corrupt)?;
                    object(&mut data)?.insert("results".into(), previous);
                    steps[step] = data;
                } else {
                    steps.push(data);
                }
            }
            "receipt" => {
                if step < step_base {
                    return Ok(());
                }
                let step = step - step_base;
                let steps = array(value, "steps")?;
                let item = steps.get_mut(step).ok_or_else(log::corrupt)?;
                let receipts = array(item, "results")?;
                if index > receipts.len() {
                    return Err(log::corrupt());
                }
                if index == receipts.len() {
                    receipts.push(data);
                } else {
                    receipts[index] = data;
                }
            }
            "guidance" => {
                if step != 0 {
                    return Err(log::corrupt());
                }
                let guidance_base = if turn == self.first {
                    self.first_guidance
                } else {
                    0
                };
                if index < guidance_base {
                    return Ok(());
                }
                let index = index - guidance_base;
                let mut data = data;
                if turn == self.first {
                    let after = field(&data, "after_step")?;
                    if after < step_base {
                        return Err(log::corrupt());
                    }
                    object(&mut data)?.insert(
                        "after_step".into(),
                        Value::Number((after - step_base).to_string()),
                    );
                }
                let guidance = array(value, "guidance")?;
                if index > guidance.len() {
                    return Err(log::corrupt());
                }
                if index == guidance.len() {
                    guidance.push(data);
                } else {
                    guidance[index] = data;
                }
            }
            "end" => {
                if step != 0 || index != 0 {
                    return Err(log::corrupt());
                }
                let fields = object(value)?;
                for key in ["end", "outcome", "metrics"] {
                    fields.insert(key.into(), data.get(key).cloned().ok_or_else(log::corrupt)?);
                }
            }
            _ => return Err(log::corrupt()),
        }
        Ok(())
    }
    pub(super) fn finish(self) -> io::Result<History> {
        let mut history = History::default();
        let count = self.turns.len();
        for (index, value) in self.turns.into_iter().enumerate() {
            let mut decoded =
                codec::decode_v2(&Value::Array(vec![value])).map_err(|_| log::corrupt())?;
            let turn = decoded.turns.pop().ok_or_else(log::corrupt)?;
            if index + 1 < count && turn.end.is_none() {
                return Err(log::corrupt());
            }
            history.turns.push(turn);
        }
        Ok(history)
    }
}
fn field(value: &Value, key: &str) -> io::Result<usize> {
    value
        .get(key)
        .and_then(Value::unsigned)
        .and_then(|n| n.try_into().ok())
        .ok_or_else(log::corrupt)
}
fn object(value: &mut Value) -> io::Result<&mut std::collections::BTreeMap<String, Value>> {
    match value {
        Value::Object(fields) => Ok(fields),
        _ => Err(log::corrupt()),
    }
}
fn array<'a>(value: &'a mut Value, key: &str) -> io::Result<&'a mut Vec<Value>> {
    match object(value)?.get_mut(key) {
        Some(Value::Array(items)) => Ok(items),
        _ => Err(log::corrupt()),
    }
}
