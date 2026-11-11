//! The next request as the model will see it: the projected conversation,
//! current instructions and the request-only environment, and its size.

use super::{Agent, environment};
use crate::{context::state_message, json::Value};

/// Starts the section of state that carries newly arrived collaborator reports.
const REPORTS: &str = "Collaborator reports, each shown once when it arrives:";
/// Starts the section of state that carries finished processes.
const PROCESSES: &str = "Processes that finished, each shown once:";

/// A state update without the sections shown only once.
fn lasting_state(text: &str) -> &str {
    let end = [REPORTS, PROCESSES]
        .iter()
        .filter_map(|section| text.find(section))
        .min()
        .unwrap_or(text.len());
    text[..end].trim_end()
}

/// The finished processes a state update carried; always its last section.
pub(super) fn process_section(text: &str) -> Option<&str> {
    text.find(PROCESSES).map(|start| text[start..].trim_end())
}

pub(super) struct Request {
    pub messages: Vec<Value>,
    pub measurement: crate::context::measurement::RequestMeasurement,
    pub estimated_tokens: usize,
    /// Instructions, runtime state and continuations sent at the end of this
    /// request, with their source position. Once the provider accepts them,
    /// they stay at that position.
    pub states: Vec<(usize, String)>,
    /// Finished processes that state carries; delivered once it is accepted.
    pub completions: Vec<crate::tools::processes::Completion>,
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
        let states = self.pending_states(&self.context, &environment, &original);
        append_states(&mut messages, &states);
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
            states: states
                .into_iter()
                .map(|text| (original.len(), text))
                .collect(),
            completions: environment.2,
        }
    }

    fn request_overhead(
        &self,
        context: &crate::context::Context,
        system: &Value,
        messages: &[Value],
        environment: &environment::Environment,
    ) -> usize {
        let pending = self.pending_states(context, environment, messages);
        let states = context
            .visible_states()
            .map(|state| state.text.as_str())
            .chain(pending.iter().map(String::as_str))
            .map(|text| state_message(text).encode().len())
            .fold(0usize, usize::saturating_add);
        system
            .encode()
            .len()
            .saturating_add(crate::tools::encoded_size_for(!self.child))
            .saturating_add(states)
    }

    /// New instructions and runtime state, or a continuation when neither
    /// changed and the request otherwise ends with a finished assistant turn.
    fn pending_states(
        &self,
        context: &crate::context::Context,
        environment: &environment::Environment,
        messages: &[Value],
    ) -> Vec<String> {
        let mut states: Vec<_> = self
            .pending_snapshot(context)
            .into_iter()
            .chain(self.pending_state(context, environment, messages))
            .collect();
        if states.is_empty() {
            let last = messages
                .iter()
                .enumerate()
                .skip(context.from)
                .rfind(|(index, _)| !context.returned.contains(index));
            let ends_with_state = context
                .visible_states()
                .last()
                .is_some_and(|state| last.is_none_or(|(index, _)| state.before > index));
            let needs_turn = !ends_with_state && (last.is_some() || context.summary.is_empty());
            if needs_turn
                && super::continuation::note_after(
                    last.map(|(_, message)| message).or(messages.first()),
                )
                .is_some()
            {
                states.push(crate::context::CONTINUATION.into());
            }
        }
        states
    }

    /// Current instructions when they differ from the latest snapshot in the
    /// window. Without one, they are sent when any apply, or when compaction
    /// summarized an earlier snapshot, so a summary cannot carry stale rules.
    fn pending_snapshot(&self, context: &crate::context::Context) -> Option<String> {
        let current = self.instruction_snapshot();
        match context.latest_state(true) {
            Some(latest) => (latest.text != current).then_some(current),
            None => (self.has_instructions()
                || context
                    .states
                    .iter()
                    .any(crate::context::RequestState::is_snapshot))
            .then_some(current),
        }
    }

    /// Current runtime state when it differs from the latest state already in
    /// the window. Unchanged state is never repeated, so request prefixes stay
    /// identical across tool calls and provider caches keep advancing. Reports
    /// join once; after compaction, those of active collaborators are sent again.
    /// Finished processes join until a request carrying them is accepted.
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
                    .any(|state| !state.is_snapshot() && state.text.contains(marker.as_str()))
            })
            .map(|(_, entry)| entry.as_str())
            .collect::<Vec<_>>();
        let latest = context
            .latest_state(false)
            .map(|state| lasting_state(&state.text));
        let completions = &environment.2;
        if reports.is_empty() && completions.is_empty() {
            return (latest != Some(base.trim_end())).then_some(base);
        }
        let mut text = base;
        if !reports.is_empty() {
            text.push_str(&format!("\n\n{REPORTS}\n{}", reports.join("\n")));
        }
        if !completions.is_empty() {
            let entries = completions
                .iter()
                .map(crate::tools::processes::Completion::report)
                .collect::<Vec<_>>();
            text.push_str(&format!("\n\n{PROCESSES}\n{}", entries.join("\n")));
        }
        Some(text)
    }

    pub(super) fn projected_context(
        &self,
        context: &crate::context::Context,
        messages: &[Value],
    ) -> Vec<Value> {
        let mut projected = context.project(messages, self.compatible_from);
        let origin = self.identity();
        for message in &mut projected {
            crate::conversation::retain_output_measurement(message, &origin);
        }
        if let Some(system) = projected.first_mut() {
            // Send the current native prompt while retaining original history verbatim.
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
                let states = self.pending_states(context, environment, messages);
                append_states(&mut projected, &states);
                context.estimate_projected_for(&projected, self.client.inputs(), !self.child)
            }
        }
    }

    /// The request projection used by compaction's before/after size check.
    /// Count both pending state kinds and any continuation note on each side.
    pub(super) fn instructed_context(
        &self,
        context: &crate::context::Context,
        environment: &environment::Environment,
        messages: &[Value],
    ) -> Vec<Value> {
        let mut projected = self.projected_context(context, messages);
        let states = self.pending_states(context, environment, messages);
        append_states(&mut projected, &states);
        projected
    }
}

/// Ends a request with newly sent state, including any anchored continuation.
fn append_states(projected: &mut Vec<Value>, states: &[String]) {
    projected.extend(states.iter().map(|state| state_message(state)));
}
