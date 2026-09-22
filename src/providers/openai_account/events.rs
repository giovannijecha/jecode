use super::{Error, Response, field, response};
use crate::{
    json::{self, Value},
    stream,
};
use std::{collections::BTreeMap, ops::ControlFlow};

#[derive(Clone, Copy)]
pub struct Limits {
    pub event_bytes: usize,
    pub output_bytes: usize,
    pub items: usize,
    pub wire_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            event_bytes: 2 * 1024 * 1024,
            output_bytes: 8 * 1024 * 1024,
            items: 128,
            wire_bytes: 64 * 1024 * 1024,
        }
    }
}

pub enum Progress<'a> {
    Text(&'a str),
    Reasoning(&'a str),
}

/// A tool call is available only through `finish`, after a validated terminal event.
pub struct ResponseStream {
    framing: stream::Decoder,
    state: State,
    error: Option<Error>,
    wire_bytes: usize,
}
struct State {
    limits: Limits,
    response_id: Option<String>,
    items: BTreeMap<usize, Value>,
    item_bytes: usize,
    preview: String,
    result: Option<Response>,
}

impl Default for ResponseStream {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}

impl ResponseStream {
    pub fn new(limits: Limits) -> Self {
        Self {
            framing: stream::Decoder::new(stream::Limits {
                line_bytes: limits.event_bytes,
                event_bytes: limits.event_bytes,
            }),
            state: State {
                limits,
                response_id: None,
                items: BTreeMap::new(),
                item_bytes: 0,
                preview: String::new(),
                result: None,
            },
            error: None,
            wire_bytes: 0,
        }
    }
    pub fn is_finished(&self) -> bool {
        self.error.is_some() || self.state.result.is_some()
    }
    pub fn push(
        &mut self,
        bytes: &[u8],
        mut progress: impl FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<(), Error> {
        if self.is_finished() {
            return Err(Error::Closed);
        }
        let Some(total) = self.wire_bytes.checked_add(bytes.len()) else {
            return self.fail(Error::Limit);
        };
        self.wire_bytes = total;
        if self.wire_bytes > self.state.limits.wire_bytes {
            return self.fail(Error::Limit);
        }
        let state = &mut self.state;
        let mut error = None;
        let framed = self.framing.push(bytes, |event| {
            if let Err(failure) = state.event(event, &mut progress) {
                error = Some(failure);
            }
            if error.is_some() || state.result.is_some() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        if let Some(error) = error {
            return self.fail(error);
        }
        if let Err(error) = framed
            && (error != stream::Error::Cancelled || self.state.result.is_none())
        {
            return self.fail(Error::Framing(error));
        }
        Ok(())
    }
    pub fn cancel(&mut self) {
        let _ = self.fail(Error::Cancelled);
    }
    fn fail(&mut self, error: Error) -> Result<(), Error> {
        self.framing.close();
        self.state.result = None;
        self.state.items.clear();
        self.state.preview.clear();
        self.error = Some(error);
        Err(error)
    }
    pub fn finish(self) -> Result<Response, Error> {
        if let Some(error) = self.error {
            return Err(error);
        }
        self.state.result.ok_or(Error::MissingTerminal)
    }
}

impl State {
    fn event(
        &mut self,
        text: &str,
        progress: &mut impl FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<(), Error> {
        if text == "[DONE]" {
            return Err(Error::MissingTerminal);
        }
        let event = json::parse(
            text,
            json::Limits {
                bytes: self.limits.event_bytes,
                ..Default::default()
            },
        )?;
        match field(&event, "type")? {
            "response.created" | "response.in_progress" => {
                self.identity(event.get("response").ok_or(Error::InvalidEvent)?)?;
            }
            "response.output_text.delta" | "response.refusal.delta" => {
                let delta = field(&event, "delta")?;
                if delta.len() > self.limits.output_bytes.saturating_sub(self.preview.len()) {
                    return Err(Error::Limit);
                }
                self.preview.push_str(delta);
                if progress(Progress::Text(delta)).is_break() {
                    return Err(Error::Cancelled);
                }
            }
            "response.reasoning_summary_text.delta"
                if progress(Progress::Reasoning(field(&event, "delta")?)).is_break() =>
            {
                return Err(Error::Cancelled);
            }
            "response.output_item.done" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::unsigned)
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or(Error::InvalidEvent)?;
                if index >= self.limits.items || self.items.contains_key(&index) {
                    return Err(Error::ConflictingOutput);
                }
                let item = event.get("item").ok_or(Error::InvalidEvent)?;
                field(item, "type")?;
                let size = json::encode(
                    item,
                    self.limits.output_bytes.saturating_sub(self.item_bytes),
                )?
                .len();
                self.item_bytes += size;
                self.items.insert(index, item.clone());
            }
            "response.completed" | "response.done" | "response.incomplete" => {
                let data = event.get("response").ok_or(Error::InvalidEvent)?;
                self.identity(data)?;
                let completed = field(&event, "type")? != "response.incomplete";
                let output = match data.get("output") {
                    None => &[][..],
                    Some(value) => value.array().ok_or(Error::InvalidEvent)?,
                };
                let output = if output.is_empty() {
                    if self
                        .items
                        .keys()
                        .enumerate()
                        .any(|(expected, actual)| expected != *actual)
                    {
                        return Err(Error::ConflictingOutput);
                    }
                    std::mem::take(&mut self.items).into_values().collect()
                } else {
                    if output.len() > self.limits.items {
                        return Err(Error::Limit);
                    }
                    for (index, item) in &self.items {
                        if output.get(*index) != Some(item) {
                            return Err(Error::ConflictingOutput);
                        }
                    }
                    output.to_vec()
                };
                json::encode(&Value::Array(output.clone()), self.limits.output_bytes)?;
                let result = response::assemble(data, output, completed)?;
                if !result.text.starts_with(&self.preview) {
                    return Err(Error::ConflictingOutput);
                }
                self.result = Some(result);
            }
            "response.failed" | "error" => return Err(Error::RemoteFailure),
            _ => {} // Bounded metadata and argument deltas are not executable output.
        }
        Ok(())
    }

    fn identity(&mut self, response: &Value) -> Result<(), Error> {
        let id = field(response, "id")?;
        if !super::identifier(id) {
            return Err(Error::InvalidEvent);
        }
        if self.response_id.as_deref().is_some_and(|known| known != id) {
            return Err(Error::ConflictingOutput);
        }
        if self.response_id.is_none() {
            self.response_id = Some(id.to_owned());
        }
        Ok(())
    }
}
