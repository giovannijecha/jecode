use crate::cancel::Cancellation;
use crate::effort::Effort;
use crate::events::{Event, EventSink};
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
    prepared: Option<crate::attachments::Prompt>,
    context: crate::context::Context,
    /// User preference, not session state: resumed conversations use the current value.
    auto_compact: u8,
    project_instructions: String,
    skills: crate::skills::Library,
    used_skills: std::collections::BTreeSet<String>,
    inbox: crate::inbox::Inbox,
    /// Saved per-model routing; `None` leaves routing unset.
    routing_preferences: Option<std::collections::BTreeMap<String, crate::openrouter::Routing>>,
    charges: Mutex<crate::openrouter::Charges>,
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
            auto_compact: crate::context::AUTO_COMPACT_DEFAULT,
            project_instructions: String::new(),
            skills: crate::skills::Library::default(),
            used_skills: std::collections::BTreeSet::new(),
            inbox: crate::inbox::Inbox::default(),
            routing_preferences: None,
            charges: Mutex::default(),
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
        if changed_model {
            client.set_routing(self.preferred_routing(client.model()));
        } else {
            client.set_router(self.client.router().clone());
        }
        self.redactor.include(client.redactor());
        self.tools.include_credentials(self.redactor.clone());
        self.client = client;
        self.sync_conversation();
        if changed_model {
            self.compatible_from = self.messages.lock().unwrap().len();
            self.context.reset_usage();
        }
    }

    /// A different model takes its saved routing preference, or Auto; the
    /// same model keeps the conversation's routing.
    pub fn set_model(&mut self, model: String) -> Result<(), String> {
        if model != self.model() {
            let routing = self.preferred_routing(&model);
            self.client.set_model(model)?;
            self.client.set_routing(routing);
            self.context.summary_output_tokens = 0;
            self.compatible_from = self.messages.lock().unwrap().len();
            self.context.reset_usage();
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        *self.charges.lock().unwrap() = crate::openrouter::Charges::default();
        self.project_instructions.clear();
        self.used_skills.clear();
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

    /// Prompts submitted while this agent works; ordinary messages join the active turn.
    pub fn inbox(&self) -> crate::inbox::Inbox {
        self.inbox.clone()
    }

    /// Observes this conversation's persistent processes while a worker owns the agent.
    pub fn processes(&self) -> crate::tools::processes::Observer {
        self.tools.observe_processes(self.redactor.clone())
    }

    pub fn run_turn(
        &mut self,
        prompt: impl Into<crate::attachments::Prompt>,
        events: &mut impl EventSink,
    ) -> Result<(), String> {
        let prompt = prompt.into();
        let mut partial = String::new();
        let result = self.turn(&prompt, &mut |event| {
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

    fn turn(
        &mut self,
        prompt: &crate::attachments::Prompt,
        events: &mut impl EventSink,
    ) -> Result<(), String> {
        if self.prepared.as_ref() != Some(prompt) {
            self.prepare_turn(prompt)?;
        }
        self.prepared = None;
        // Endpoint refresh may wait on the network. Run it here on the
        // execution worker, not while the TUI prepares the submitted prompt.
        self.prepare_routing()?;
        let limits = self.model_limits(events)?;
        self.emit_context(events)?;
        loop {
            if self.cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            let completion = self.next_completion(limits, events)?;
            let mut message = completion.message;
            let annotation_error =
                crate::attachments::annotations::store(&mut message, self.tools.attachments())
                    .err();
            if annotation_error.is_some()
                && let Value::Object(fields) = &mut message
            {
                fields.remove("annotations");
            }
            self.messages.lock().unwrap().push(message);
            if let Err(error) = self.checkpoint(crate::sessions::Stage::Completion) {
                self.cancel_calls(&completion.calls);
                return Err(error);
            }
            let shown = if completion.text.is_empty() {
                Ok(())
            } else {
                events.emit(Event::Message {
                    text: self.redact(&completion.text),
                })
            };
            if let Err(error) = shown.and_then(|()| self.emit_context(events)) {
                self.cancel_calls(&completion.calls);
                return Err(error);
            }
            if let Some(error) = annotation_error {
                self.cancel_calls(&completion.calls);
                return Err(format!("Could not store PDF annotations: {error}"));
            }
            if completion.calls.is_empty() {
                if self.continue_turn(events)? {
                    continue;
                }
                return Ok(());
            }
            self.execute_calls(&completion.calls, events)?;
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
        if let Some(reference) = result.get("skill_reference").and_then(Value::as_str)
            && let Ok((name, _)) = crate::skills::parse_reference(reference)
        {
            self.used_skills.insert(name.to_owned());
        }
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
            context.estimate(messages, self.compatible_from, self.client.inputs())
        } else {
            context.estimate_bytes(
                self.context_bytes(&self.projected_context(context, messages))
                    .saturating_add(crate::tools::definitions().encode().len()),
            )
        }
    }

    fn context_bytes(&self, messages: &[Value]) -> usize {
        crate::context::bytes_for(messages, self.client.inputs())
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

pub(crate) mod checkpoints;
mod compaction;
mod environment;
mod execution;
mod history;
mod input;
mod persistence;
mod project_instructions;
mod recovery;
mod routing;
mod skills;
mod summary;
mod temporary;

#[cfg(test)]
mod attachment_tests;
#[cfg(test)]
mod checkpoint_tests;
#[cfg(test)]
mod compaction_tests;
#[cfg(test)]
mod context_status_tests;
#[cfg(test)]
mod context_tests;
#[cfg(test)]
mod delete_tests;
#[cfg(test)]
mod history_tests;
#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod instruction_tests;
#[cfg(test)]
mod local_read_tests;
#[cfg(test)]
mod long_work_tests;
#[cfg(test)]
mod persistence_tests;
#[cfg(test)]
mod process_tests;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod routing_tests;
#[cfg(test)]
mod skills_tests;
#[cfg(test)]
mod summary_tests;
#[cfg(test)]
mod tests;
