use crate::cancel::Cancellation;
use crate::effort::Effort;
use crate::events::{Event, EventSink, tool_summary};
use crate::export::{Archive, Messages};
use crate::json::Value;
use crate::openrouter::{OpenRouter, ToolCall};
use crate::tools::Tools;
use std::sync::{Arc, Mutex};

pub struct Agent {
    client: OpenRouter,
    tools: Tools,
    messages: Messages,
    cancellation: Cancellation,
    compatible_from: usize,
    redactor: crate::redact::Redactor,
    events: Messages,
    persistence: Option<crate::sessions::Handle>,
    prepared: Option<String>,
    context: crate::context::Context,
    project_instructions: String,
}

impl Agent {
    pub fn new(mut client: OpenRouter, tools: Tools) -> Self {
        let cancellation = Cancellation::default();
        client.set_cancellation(cancellation.clone());
        let redactor = client.redactor();
        let mut tools = tools;
        tools.include_credentials(redactor.clone());
        let mut agent = Self {
            client,
            tools,
            messages: Arc::new(Mutex::new(Vec::new())),
            cancellation,
            compatible_from: 0,
            redactor,
            events: Arc::new(Mutex::new(Vec::new())),
            persistence: None,
            prepared: None,
            context: crate::context::Context::default(),
            project_instructions: String::new(),
        };
        agent.clear();
        agent
    }

    pub fn model(&self) -> &str {
        self.client.model()
    }

    pub fn effort(&self) -> Effort {
        self.client.effort()
    }

    pub fn set_effort(&mut self, effort: Effort) {
        if effort != self.effort() {
            self.context.summary_output_tokens = 0;
        }
        self.client.set_effort(effort);
    }

    pub fn api(&self) -> crate::openrouter::Api {
        self.client.api()
    }

    pub fn redact(&self, text: &str) -> String {
        self.redactor.text(text)
    }

    pub fn replace_client(&mut self, mut client: OpenRouter) {
        let changed_model = self.model() != client.model();
        if changed_model || self.effort() != client.effort() {
            self.context.summary_output_tokens = 0;
        }
        client.set_cancellation(self.cancellation.clone());
        self.redactor.include(client.redactor());
        self.tools.include_credentials(self.redactor.clone());
        self.client = client;
        if changed_model {
            self.compatible_from = self.messages.lock().unwrap().len();
            self.context.reset_usage();
        }
    }

    pub fn set_model(&mut self, model: String) -> Result<(), String> {
        if model != self.model() {
            self.client.set_model(model)?;
            self.context.summary_output_tokens = 0;
            self.compatible_from = self.messages.lock().unwrap().len();
            self.context.reset_usage();
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.project_instructions.clear();
        self.prepared = None;
        self.context = crate::context::Context::default();
        self.compatible_from = 0;
        self.events.lock().unwrap().clear();
        *self.messages.lock().unwrap() = vec![self.system_message()];
    }

    pub fn archive(&self) -> Archive {
        Archive {
            model: self.model().into(),
            directory: self.tools.root().to_path_buf(),
            messages: Arc::clone(&self.messages),
            redactor: self.redactor.clone(),
            effort: self.effort().name().into(),
            events: Arc::clone(&self.events),
        }
    }

    pub fn cancellation(&self) -> Cancellation {
        self.cancellation.clone()
    }

    pub fn run_turn(&mut self, prompt: &str, events: &mut impl EventSink) -> Result<(), String> {
        let mut partial = String::new();
        let result = self.turn(prompt, &mut |event| {
            match &event {
                Event::Streaming { text } => partial = text.clone(),
                Event::Message { .. }
                | Event::Waiting { .. }
                | Event::Recovering { .. }
                | Event::RequestDiscarded => partial.clear(),
                _ => {}
            }
            events.emit(event)
        });
        if let Err(error) = &result {
            self.record_turn_error(error, &partial);
        }
        let saved = self.checkpoint(crate::sessions::Stage::Ready);
        if result.is_ok()
            && let Err(error) = &saved
        {
            self.record_turn_error(error, "");
            let _ = self.checkpoint(crate::sessions::Stage::Ready);
        }
        result.and(saved)
    }

    fn turn(&mut self, prompt: &str, events: &mut impl EventSink) -> Result<(), String> {
        if self.prepared.as_deref() != Some(prompt) {
            self.prepare_turn(prompt)?;
        }
        self.prepared = None;
        let limits = self.model_limits(events)?;
        loop {
            if self.cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            let completion = self.next_completion(limits, events)?;
            self.messages.lock().unwrap().push(completion.message);
            if let Err(error) = self.checkpoint(crate::sessions::Stage::Completion) {
                self.cancel_calls(&completion.calls);
                return Err(error);
            }
            if !completion.text.is_empty()
                && let Err(error) = events.emit(Event::Message {
                    text: self.redact(&completion.text),
                })
            {
                self.cancel_calls(&completion.calls);
                return Err(error);
            }
            if completion.calls.is_empty() {
                return Ok(());
            }
            for (index, call) in completion.calls.iter().enumerate() {
                if self.cancellation.requested() {
                    self.cancel_calls(&completion.calls[index..]);
                    return Err("Operation cancelled".into());
                }
                let arguments = crate::json::parse(&call.arguments)
                    .unwrap_or_else(|_| Value::string(&call.arguments));
                if let Err(error) = self.checkpoint(crate::sessions::Stage::Tool(call.id.clone())) {
                    self.cancel_calls(&completion.calls[index..]);
                    return Err(error);
                }
                if let Err(error) = events.emit(Event::ToolStarted {
                    id: self.redact(&call.id),
                    name: self.redact(&call.name),
                    arguments: self.redactor.value(&arguments),
                }) {
                    self.cancel_calls(&completion.calls[index..]);
                    return Err(error);
                }
                let result = self.execute_tool(call);
                self.tool_result(call, &result);
                if let Err(error) = self.checkpoint(crate::sessions::Stage::ToolFinished) {
                    self.cancel_calls(&completion.calls[index + 1..]);
                    return Err(error);
                }
                if let Err(error) = events.emit(Event::ToolFinished {
                    id: self.redact(&call.id),
                    name: self.redact(&call.name),
                    summary: self.redact(&tool_summary(&call.name, &result)),
                    result: self.redactor.value(&result),
                }) {
                    self.cancel_calls(&completion.calls[index + 1..]);
                    return Err(error);
                }
            }
        }
    }

    fn tool_result(&mut self, call: &ToolCall, result: &Value) {
        self.messages.lock().unwrap().push(Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string(&call.id)),
            (
                "content",
                Value::string(self.redactor.value(result).encode()),
            ),
        ]));
    }

    fn cancel_calls(&mut self, calls: &[ToolCall]) {
        for call in calls {
            self.tool_result(
                call,
                &Value::object([(
                    "error",
                    Value::string("Tool was not executed because the turn was interrupted"),
                )]),
            );
        }
    }

    fn transport_messages(&self) -> Vec<Value> {
        let original = self.messages.lock().unwrap();
        let mut messages = self.projected_context(&self.context, &original);
        let temporary = self.request_environment();
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

    fn projected_context(
        &self,
        context: &crate::context::Context,
        messages: &[Value],
    ) -> Vec<Value> {
        let mut projected = context.project(messages, self.compatible_from);
        if let Some(system) = projected.first_mut() {
            // Send current native instructions while retaining original history verbatim.
            *system = self.current_system_message();
        }
        projected
    }

    fn projected_estimate(&self, context: &crate::context::Context, messages: &[Value]) -> usize {
        if context.input_tokens.is_some()
            && context.measured_end >= context.from
            && context.measured_end <= messages.len()
        {
            context.estimate(messages, self.compatible_from)
        } else {
            context.estimate_bytes(
                crate::context::bytes(&self.projected_context(context, messages))
                    .saturating_add(crate::tools::definitions().encode().len()),
            )
        }
    }

    fn context_estimate(&self, messages: &[Value]) -> usize {
        let estimate = self.projected_estimate(&self.context, messages);
        if self.context.input_tokens.is_some()
            && self.context.measured_end >= self.context.from
            && self.context.measured_end <= messages.len()
        {
            estimate
        } else {
            estimate.saturating_add(self.request_environment().len())
        }
    }

    pub fn record_local(&self, command: &str, result: &str) {
        self.record_local_details(command, result, "notice", &[]);
    }

    pub fn record_local_details(
        &self,
        command: &str,
        result: &str,
        kind: &str,
        details: &[(String, String)],
    ) {
        self.events.lock().unwrap().push(Value::object([
            ("type", Value::string("local_command")),
            ("command", Value::string(self.redact(command))),
            ("result", Value::string(self.redact(result))),
            ("kind", Value::string(kind)),
            (
                "details",
                Value::Array(
                    details
                        .iter()
                        .map(|(key, value)| {
                            Value::Array(vec![
                                Value::string(self.redact(key)),
                                Value::string(self.redact(value)),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("model", Value::string(self.model())),
            ("effort", Value::string(self.effort().name())),
            (
                "after_message",
                Value::number(self.messages.lock().unwrap().len()),
            ),
        ]));
    }
}

mod compaction;
mod environment;
mod history;
mod persistence;
mod project_instructions;
mod recovery;
mod summary;
mod temporary;

#[cfg(test)]
mod compaction_tests;
#[cfg(test)]
mod context_tests;
#[cfg(test)]
mod delete_tests;
#[cfg(test)]
mod history_tests;
#[cfg(test)]
mod instruction_tests;
#[cfg(test)]
mod long_work_tests;
#[cfg(test)]
mod persistence_tests;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod summary_tests;
#[cfg(test)]
mod tests;
