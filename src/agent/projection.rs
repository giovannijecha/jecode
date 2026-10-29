//! The next request as the model will see it: the projected conversation,
//! current instructions and the request-only environment, and its size.

use super::{Agent, environment};
use crate::json::Value;

impl Agent {
    pub(super) fn transport_messages(&self) -> Vec<Value> {
        let environment = self.environment();
        let original = self.messages.lock().unwrap();
        let mut messages = self.projected_context(&self.context, &original);
        let temporary = self.request_environment_for(&environment, &original);
        if !temporary.is_empty()
            && let Some(Value::Object(system)) = messages.first_mut()
        {
            let original = system.get("content").and_then(Value::as_str).unwrap_or("");
            system.insert(
                "content".into(),
                Value::string(format!("{original}\n\n{temporary}")),
            );
        }
        messages
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
        if let Some(system) = projected.first_mut() {
            // Send current native instructions while retaining original history verbatim.
            *system = self.current_system_message();
        }
        projected
    }

    pub(super) fn projected_estimate(
        &self,
        context: &crate::context::Context,
        messages: &[Value],
    ) -> usize {
        match context.measured_input(messages, self.client.inputs()) {
            Some(tokens) => context.estimate_growth(messages, tokens, self.client.inputs()),
            None => self.unmeasured_estimate(context, messages),
        }
    }

    pub(super) fn unmeasured_estimate(
        &self,
        context: &crate::context::Context,
        messages: &[Value],
    ) -> usize {
        context.estimate_projected_for(
            &self.projected_context(context, messages),
            self.client.inputs(),
            !self.child,
        )
    }

    pub(super) fn context_bytes(&self, messages: &[Value]) -> usize {
        crate::context::bytes_for(messages, self.client.inputs())
    }

    pub(super) fn context_estimate(
        &self,
        environment: &environment::Environment,
        messages: &[Value],
    ) -> usize {
        match self.context.measured_input(messages, self.client.inputs()) {
            Some(tokens) => self
                .context
                .estimate_growth(messages, tokens, self.client.inputs()),
            None => self
                .unmeasured_estimate(&self.context, messages)
                .saturating_add(self.request_environment_for(environment, messages).len()),
        }
    }
}
