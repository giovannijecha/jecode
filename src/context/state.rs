//! Runtime state at fixed positions. Each update stays where it was first
//! sent, so a later one replaces it in meaning while every earlier request
//! remains an identical, reusable prefix for provider prompt caches.

use super::{Context, Value};

const STATE_PREFIX: &str = "Jecode session state, current until a later session state update. This is runtime context, not a new user task. Continue the existing task using the current state below.";

/// Runtime state the model saw at a fixed point: before source message `before`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestState {
    pub before: usize,
    pub text: String,
}

/// The request-only message that carries runtime state.
pub fn state_message(text: &str) -> Value {
    Value::object([
        ("role", Value::string("user")),
        ("jecode_request_state", Value::Bool(true)),
        (
            "content",
            Value::string(format!("{STATE_PREFIX}\n\n{text}")),
        ),
    ])
}

pub(super) fn value(states: &[RequestState]) -> Value {
    Value::Array(
        states
            .iter()
            .map(|state| {
                Value::object([
                    ("before", Value::number(state.before)),
                    ("text", Value::string(&state.text)),
                ])
            })
            .collect(),
    )
}

/// Sessions saved before anchored state send it again once, at the end.
pub(super) fn parse(value: Option<&Value>) -> Result<Vec<RequestState>, String> {
    let Some(states) = value else {
        return Ok(Vec::new());
    };
    states
        .as_array()
        .ok_or("Invalid saved request states")?
        .iter()
        .map(|state| {
            Some(RequestState {
                before: state.get("before")?.as_usize()?,
                text: state.get("text")?.as_str()?.to_owned(),
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| "Invalid saved request state".into())
}

impl Context {
    /// State updates inside the current window, in the order they were sent.
    pub fn visible_states(&self) -> impl Iterator<Item = &RequestState> {
        self.states
            .iter()
            .filter(move |state| state.before >= self.from)
    }

    /// Records state first sent before source message `before`. Unchanged
    /// state is already visible, so it is not repeated.
    pub fn anchor_state(&mut self, before: usize, text: String) {
        if self
            .visible_states()
            .last()
            .map(|state| state.text.as_str())
            != Some(text.as_str())
        {
            self.states.retain(|state| state.before <= before);
            self.states.push(RequestState { before, text });
        }
    }
}
