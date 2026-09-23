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
    indexed: Option<bool>,
    last_text: Option<(usize, usize)>,
    parts: BTreeMap<(usize, usize), PartProgress>,
    done_bytes: usize,
    last_sequence: Option<u64>,
    result: Option<Response>,
}
#[derive(Default)]
struct PartProgress {
    text: String,
    item_id: Option<String>,
    done: Option<String>,
    done_events: u8,
}
struct TextTarget<'a> {
    output_index: usize,
    content_index: usize,
    item_id: Option<&'a str>,
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
                indexed: None,
                last_text: None,
                parts: BTreeMap::new(),
                done_bytes: 0,
                last_sequence: None,
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
        self.state.parts.clear();
        self.state.done_bytes = 0;
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
        if let Some(sequence) = event.get("sequence_number") {
            let sequence = sequence.unsigned().ok_or(Error::InvalidEvent)?;
            if self.last_sequence.is_some_and(|last| sequence <= last) {
                return Err(Error::ConflictingOutput);
            }
            self.last_sequence = Some(sequence);
        }
        match field(&event, "type")? {
            "response.created" | "response.in_progress" => {
                self.identity(event.get("response").ok_or(Error::InvalidEvent)?)?;
            }
            "response.output_text.delta" | "response.refusal.delta" => {
                self.text_delta(&event, progress)?;
            }
            "response.output_text.done" | "response.refusal.done" => {
                let key = if field(&event, "type")? == "response.output_text.done" {
                    "text"
                } else {
                    "refusal"
                };
                self.text_done(&event, field(&event, key)?, 1)?;
            }
            "response.content_part.done" => {
                let part = event.get("part").ok_or(Error::InvalidEvent)?;
                match field(part, "type")? {
                    "output_text" => self.text_done(&event, field(part, "text")?, 2)?,
                    "refusal" => self.text_done(&event, field(part, "refusal")?, 2)?,
                    _ => {}
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
                self.validate_text(&result)?;
                self.result = Some(result);
            }
            "response.failed" | "error" => return Err(Error::RemoteFailure),
            _ => {} // Bounded metadata and argument deltas are not executable output.
        }
        Ok(())
    }

    fn target<'a>(&self, event: &'a Value) -> Result<Option<TextTarget<'a>>, Error> {
        let indices = match (event.get("output_index"), event.get("content_index")) {
            (None, None) => return Ok(None),
            (Some(output), Some(content)) => (output, content),
            _ => return Err(Error::InvalidEvent),
        };
        let index = |value: &Value| {
            value
                .unsigned()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n < self.limits.items)
                .ok_or(Error::InvalidEvent)
        };
        let item_id = event
            .get("item_id")
            .map(|value| value.text().ok_or(Error::InvalidEvent))
            .transpose()?;
        if item_id.is_some_and(|id| !super::identifier(id)) {
            return Err(Error::InvalidEvent);
        }
        Ok(Some(TextTarget {
            output_index: index(indices.0)?,
            content_index: index(indices.1)?,
            item_id,
        }))
    }

    fn text_delta(
        &mut self,
        event: &Value,
        progress: &mut impl FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<(), Error> {
        let delta = field(event, "delta")?;
        let target = self.target(event)?;
        let indexed = target.is_some();
        if self.indexed.is_some_and(|known| known != indexed) {
            return Err(Error::ConflictingOutput);
        }
        self.indexed = Some(indexed);
        let separator = if let Some(TextTarget {
            output_index,
            content_index,
            item_id,
        }) = target
        {
            let target = (output_index, content_index);
            if self.last_text.is_some_and(|last| target < last) {
                return Err(Error::ConflictingOutput);
            }
            let part = self.parts.entry(target).or_default();
            if part.done.is_some()
                || part
                    .item_id
                    .as_deref()
                    .zip(item_id)
                    .is_some_and(|(a, b)| a != b)
            {
                return Err(Error::ConflictingOutput);
            }
            if part.item_id.is_none() {
                part.item_id = item_id.map(str::to_owned);
            }
            part.text.push_str(delta);
            let separator = if !delta.is_empty() && !self.preview.is_empty() {
                match self.last_text {
                    Some((last_output, last_content)) if target != (last_output, last_content) => {
                        if last_output == output_index {
                            "\n"
                        } else {
                            "\n\n"
                        }
                    }
                    _ => "",
                }
            } else {
                ""
            };
            if !delta.is_empty() {
                self.last_text = Some(target);
            }
            separator
        } else {
            ""
        };
        if delta.len() + separator.len()
            > self.limits.output_bytes.saturating_sub(self.preview.len())
        {
            return Err(Error::Limit);
        }
        if !separator.is_empty() {
            self.preview.push_str(separator);
            if progress(Progress::Text(separator)).is_break() {
                return Err(Error::Cancelled);
            }
        }
        self.preview.push_str(delta);
        if progress(Progress::Text(delta)).is_break() {
            return Err(Error::Cancelled);
        }
        Ok(())
    }

    fn text_done(&mut self, event: &Value, text: &str, kind: u8) -> Result<(), Error> {
        let Some(TextTarget {
            output_index,
            content_index,
            item_id,
        }) = self.target(event)?
        else {
            // The account endpoint is not covered by the public API contract.
            return Ok(());
        };
        let part = self.parts.entry((output_index, content_index)).or_default();
        if part.done_events & kind != 0
            || !text.starts_with(&part.text)
            || part.done.as_deref().is_some_and(|done| done != text)
            || part
                .item_id
                .as_deref()
                .zip(item_id)
                .is_some_and(|(a, b)| a != b)
        {
            return Err(Error::ConflictingOutput);
        }
        if part.item_id.is_none() {
            part.item_id = item_id.map(str::to_owned);
        }
        if part.done.is_none() {
            if text.len() > self.limits.output_bytes.saturating_sub(self.done_bytes) {
                return Err(Error::Limit);
            }
            self.done_bytes += text.len();
            part.done = Some(text.to_owned());
        }
        part.done_events |= kind;
        Ok(())
    }

    fn validate_text(&self, result: &Response) -> Result<(), Error> {
        let parts = response::text_parts(&result.output)?;
        for ((output, content), progress) in &self.parts {
            let part = parts
                .iter()
                .find(|part| part.output_index == *output && part.content_index == *content)
                .ok_or(Error::ConflictingOutput)?;
            if !part.text.starts_with(&progress.text)
                || progress
                    .done
                    .as_deref()
                    .is_some_and(|done| done != part.text)
                || progress
                    .item_id
                    .as_deref()
                    .zip(part.item_id)
                    .is_some_and(|(a, b)| a != b)
            {
                return Err(Error::ConflictingOutput);
            }
        }
        if self.indexed != Some(true) {
            let raw: String = parts.iter().map(|part| part.text).collect();
            if !raw.starts_with(&self.preview) {
                return Err(Error::ConflictingOutput);
            }
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
