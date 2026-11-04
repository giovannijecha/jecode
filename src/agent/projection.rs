//! The next request as the model will see it: the projected conversation,
//! current instructions and the request-only environment, and its size.

use super::{Agent, environment};
use crate::{context::state_message, json::Value};

/// Starts the section of state that carries newly arrived collaborator reports.
const REPORTS: &str = "Collaborator reports, each shown once when it arrives:";

pub(super) struct Request {
    pub messages: Vec<Value>,
    pub measurement: crate::context::measurement::RequestMeasurement,
    pub estimated_tokens: usize,
    /// Runtime state sent at the end of this request, with its source position.
    /// Once the provider accepts the request, it stays at that position.
    pub state: Option<(usize, String)>,
}

impl Agent {
    #[cfg(test)]
    pub(super) fn transport_messages(&self) -> Vec<Value> {
        self.transport_request().messages
    }

    pub(super) fn transport_request(&self) -> Request {
        let environment = self.environment();
        let original = self.messages.lock().unwrap();
        let mut messages = self.projected_context(&self.context, &original);
        let overhead = self.request_overhead(&self.context, &messages[0], &original, &environment);
        let state = self.pending_state(&self.context, &environment, &original);
        append_state(&mut messages, state.as_deref());
        let measurement = self.context.request_measurement(
            &original,
            &messages,
            self.client.inputs(),
            !self.child,
            overhead,
        );
        Request {
            messages,
            measurement,
            estimated_tokens: self.context_estimate(&environment, &original),
            state: state.map(|text| (original.len(), text)),
        }
    }

    fn request_overhead(
        &self,
        context: &crate::context::Context,
        system: &Value,
        messages: &[Value],
        environment: &environment::Environment,
    ) -> usize {
        let pending = self.pending_state(context, environment, messages);
        let states = context
            .visible_states()
            .map(|state| state.text.as_str())
            .chain(pending.as_deref())
            .map(|text| state_message(text).encode().len())
            .fold(0usize, usize::saturating_add);
        let last = messages
            .iter()
            .enumerate()
            .skip(context.from)
            .filter(|(index, _)| !context.returned.contains(index))
            .map(|(_, message)| message)
            .next_back();
        let note = if pending.is_none() && (context.summary.is_empty() || last.is_some()) {
            super::continuation::note_after(last.or(Some(system)))
        } else {
            None
        };
        system
            .encode()
            .len()
            .saturating_add(crate::tools::encoded_size_for(!self.child))
            .saturating_add(states)
            .saturating_add(note.map_or(0, |note| note.encode().len()))
    }

    /// Current runtime state when it differs from the latest state already in
    /// the window. Unchanged state is never repeated, so request prefixes stay
    /// identical across tool calls and provider caches keep advancing. Reports
    /// join once; after compaction, those of active collaborators are sent again.
    fn pending_state(
        &self,
        context: &crate::context::Context,
        environment: &environment::Environment,
        messages: &[Value],
    ) -> Option<String> {
        let base = self.request_environment_for(environment, messages);
        let reports = environment
            .1
            .iter()
            .filter(|(marker, _)| {
                !context
                    .visible_states()
                    .any(|state| state.text.contains(marker.as_str()))
            })
            .map(|(_, entry)| entry.as_str())
            .collect::<Vec<_>>();
        let latest = context.visible_states().last().map(|state| {
            state
                .text
                .split_once(REPORTS)
                .map_or(state.text.as_str(), |(base, _)| base)
                .trim_end()
        });
        if reports.is_empty() {
            return (latest != Some(base.trim_end())).then_some(base);
        }
        Some(format!("{base}\n\n{REPORTS}\n{}", reports.join("\n")))
    }

    pub(super) fn projected_context(
        &self,
        context: &crate::context::Context,
        messages: &[Value],
    ) -> Vec<Value> {
        let stamped;
        let messages = if let Some(origin) = &self.legacy_origin {
            stamped = messages
                .iter()
                .cloned()
                .map(|mut message| {
                    if message.get("role").and_then(Value::as_str) == Some("assistant") {
                        crate::conversation::stamp(&mut message, origin);
                    }
                    message
                })
                .collect::<Vec<_>>();
            &stamped
        } else {
            messages
        };
        let mut projected = context.project(messages, self.compatible_from);
        let origin = self.identity();
        for message in &mut projected {
            crate::conversation::retain_output_measurement(message, &origin);
        }
        if let Some(system) = projected.first_mut() {
            // Send current native instructions while retaining original history verbatim.
            *system = self.current_system_message();
        }
        projected
    }

    pub(super) fn context_bytes(&self, messages: &[Value]) -> usize {
        crate::context::bytes_for(messages, self.client.inputs())
    }

    pub(super) fn context_estimate(
        &self,
        environment: &environment::Environment,
        messages: &[Value],
    ) -> usize {
        self.context_estimate_for(&self.context, environment, messages)
    }

    pub(super) fn context_estimate_for(
        &self,
        context: &crate::context::Context,
        environment: &environment::Environment,
        messages: &[Value],
    ) -> usize {
        let system = self.current_system_message();
        match context.measured_input(messages, self.client.inputs()) {
            Some(tokens) => {
                let estimate = context.estimate_growth(messages, tokens, self.client.inputs());
                context.measured_overhead_bytes.map_or(estimate, |before| {
                    context.adjust_system(
                        estimate,
                        before,
                        self.request_overhead(context, &system, messages, environment),
                    )
                })
            }
            None => {
                let mut projected = self.projected_context(context, messages);
                let state = self.pending_state(context, environment, messages);
                append_state(&mut projected, state.as_deref());
                context.estimate_projected_for(&projected, self.client.inputs(), !self.child)
            }
        }
    }
}

/// Ends a request with newly sent state, then the user turn it may need.
fn append_state(projected: &mut Vec<Value>, state: Option<&str>) {
    if let Some(state) = state {
        projected.push(state_message(state));
    }
    super::continuation::end_with_user_turn(projected);
}
