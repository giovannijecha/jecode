//! The next request as the model will see it: the projected conversation,
//! current instructions and the request-only environment, and its size.

use super::{Agent, environment};
use crate::json::Value;

pub(super) struct Request {
    pub messages: Vec<Value>,
    pub measurement: crate::context::measurement::RequestMeasurement,
    pub estimated_tokens: usize,
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
        if let Some(system) = messages.first_mut() {
            *system = self.request_system_message(&environment, &original);
        }
        let overhead = self.request_overhead(&self.context, &messages[0], &original);
        super::continuation::end_with_user_turn(&mut messages);
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
        }
    }

    fn request_overhead(
        &self,
        context: &crate::context::Context,
        system: &Value,
        messages: &[Value],
    ) -> usize {
        let last = messages
            .iter()
            .enumerate()
            .skip(context.from)
            .filter(|(index, _)| !context.returned.contains(index))
            .map(|(_, message)| message)
            .next_back();
        let note = if context.summary.is_empty() || last.is_some() {
            super::continuation::note_after(last.or(Some(system)))
        } else {
            None
        };
        system
            .encode()
            .len()
            .saturating_add(crate::tools::encoded_size_for(!self.child))
            .saturating_add(note.map_or(0, |note| note.encode().len()))
    }

    fn request_system_message(
        &self,
        environment: &environment::Environment,
        messages: &[Value],
    ) -> Value {
        let mut system = self.current_system_message();
        let temporary = self.request_environment_for(environment, messages);
        if !temporary.is_empty()
            && let Value::Object(fields) = &mut system
        {
            let native = fields.get("content").and_then(Value::as_str).unwrap_or("");
            fields.insert(
                "content".into(),
                Value::string(format!("{native}\n\n{temporary}")),
            );
        }
        system
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
        let system = self.request_system_message(environment, messages);
        match context.measured_input(messages, self.client.inputs()) {
            Some(tokens) => {
                let estimate = context.estimate_growth(messages, tokens, self.client.inputs());
                context.measured_overhead_bytes.map_or(estimate, |before| {
                    context.adjust_system(
                        estimate,
                        before,
                        self.request_overhead(context, &system, messages),
                    )
                })
            }
            None => {
                let mut projected = self.projected_context(context, messages);
                if let Some(first) = projected.first_mut() {
                    *first = system;
                }
                super::continuation::end_with_user_turn(&mut projected);
                context.estimate_projected_for(&projected, self.client.inputs(), !self.child)
            }
        }
    }
}
