//! Runtime state at fixed positions. Each update stays where it was first
//! sent, so a later one replaces it in meaning while every earlier request
//! remains an identical, reusable prefix for provider prompt caches.
//! Instruction snapshots and ordinary session state are two independent
//! kinds: each update replaces only the latest one of its own kind.
//! Continuations are anchored separately and do not replace either kind.

use super::{Context, Value};

const STATE_PREFIX: &str = "Jecode session state, current until a later session state update. This is runtime context, not a new user task. Continue the existing task using the current state below.";

pub(crate) const CONTINUATION: &str =
    "No new user message. Continue from the current session state.";

/// Opens every instruction snapshot, the complete project instructions and
/// skill catalog. Saved states keep their text, so the header is their kind.
pub const SNAPSHOT_HEADER: &str = "Jecode instruction snapshot. It replaces every earlier instruction snapshot and stays current until a later one; session state updates do not change it. Each project instruction source applies only to its stated directory scope, and more specific directory instructions take precedence on conflicts. Explicit user requests take precedence over project instructions and skills; Jecode's tool contracts still apply.";

/// Runtime state the model saw at a fixed point: before source message `before`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestState {
    pub before: usize,
    pub text: String,
}

impl RequestState {
    /// Whether this carries instructions rather than ordinary session state.
    pub fn is_snapshot(&self) -> bool {
        self.text.starts_with(SNAPSHOT_HEADER)
    }
}

/// The anchored message that carries runtime state. Instruction snapshots
/// and continuations already have their own wording.
pub fn state_message(text: &str) -> Value {
    let content = if text.starts_with(SNAPSHOT_HEADER) || text == CONTINUATION {
        text.to_owned()
    } else {
        format!("{STATE_PREFIX}\n\n{text}")
    };
    Value::object([
        ("role", Value::string("user")),
        ("jecode_request_state", Value::Bool(true)),
        ("content", Value::string(content)),
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

pub(super) fn parse(value: Option<&Value>) -> Result<Vec<RequestState>, String> {
    let states = value.ok_or("Missing saved request states")?;
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

    /// The latest visible instruction snapshot or ordinary session state;
    /// continuations do not replace either.
    pub fn latest_state(&self, snapshot: bool) -> Option<&RequestState> {
        self.visible_states()
            .filter(|state| state.is_snapshot() == snapshot && state.text != CONTINUATION)
            .last()
    }

    /// Records state first sent before source message `before`. Unchanged
    /// state of the same kind is already visible, so it is not repeated.
    pub fn anchor_state(&mut self, before: usize, text: String) {
        let snapshot = text.starts_with(SNAPSHOT_HEADER);
        // Each continuation has its own position, even when its text repeats.
        // It does not replace the latest runtime state or instruction snapshot.
        let unchanged = if text == CONTINUATION {
            self.states
                .iter()
                .any(|state| state.before == before && state.text == text)
        } else {
            self.latest_state(snapshot).map(|state| state.text.as_str()) == Some(text.as_str())
        };
        if !unchanged {
            self.states.retain(|state| state.before <= before);
            self.states.push(RequestState { before, text });
        }
    }
}
