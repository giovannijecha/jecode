#[cfg(test)]
use crate::backend::openrouter::OpenRouter;
use crate::backend::{Backend, ToolCall};
use crate::cancel::Cancellation;
use crate::effort::Effort;
use crate::events::{Event, EventSink};
use crate::export::{Archive, Messages};
use crate::json::Value;
use crate::tools::Tools;
pub use retraction::TurnInput;
use std::sync::{Arc, Mutex};

pub struct Agent {
    client: Box<dyn Backend>,
    accounts: std::collections::BTreeMap<String, crate::backend::Services>,
    tools: Tools,
    messages: Messages,
    cancellation: Cancellation,
    compatible_from: usize,
    legacy_origin: Option<crate::conversation::Origin>,
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
    routing_preferences:
        Option<std::collections::BTreeMap<(String, String), crate::backend::Routing>>,
    charges: Mutex<crate::backend::Charges>,
}

impl Agent {
    #[cfg(test)]
    fn fixture_capacity(&self, headroom: usize, auto_percent: u8) -> usize {
        // Keep fixture budgets relative to native prompt/tool overhead, not
        // to the conversation whose compaction the test exercises.
        let fixed = self.current_system_message().encode().len()
            + crate::tools::encoded_size()
            + self.request_environment().len();
        (fixed + headroom)
            .saturating_mul(100)
            .div_ceil(usize::from(auto_percent))
    }

    #[cfg(test)]
    pub fn new<B: Backend + 'static>(client: B, tools: Tools) -> Self {
        Self::with_backend(Box::new(client), tools)
    }

    pub fn with_backend(mut client: Box<dyn Backend>, tools: Tools) -> Self {
        let cancellation = Cancellation::default();
        client.set_cancellation(cancellation.clone());
        let redactor = client.redactor();
        let mut tools = tools;
        tools.include_credentials(redactor.clone());
        let mut agent = Self {
            client,
            accounts: std::collections::BTreeMap::new(),
            tools,
            messages: Arc::new(Mutex::new(Vec::new())),
            cancellation,
            compatible_from: 0,
            legacy_origin: None,
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
        agent.register_backend(agent.client.services());
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

    pub fn redact(&self, text: &str) -> String {
        self.redactor.text(text)
    }

    #[cfg(test)]
    pub fn replace_client<B: Backend + 'static>(&mut self, client: B) {
        self.replace_backend(Box::new(client)).unwrap();
    }

    /// A different model takes its saved routing preference, or Auto; the
    /// same model keeps the conversation's routing.
    pub fn set_model(&mut self, model: String) -> Result<(), String> {
        if model != self.model() {
            let mut next = self.services().create(model)?;
            next.set_effort(self.effort());
            self.install_backend(next, false)?;
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        *self.charges.lock().unwrap() = crate::backend::Charges::default();
        self.project_instructions.clear();
        self.used_skills.clear();
        self.prepared = None;
        self.context = crate::context::Context::default();
        self.compatible_from = 0;
        self.legacy_origin = None;
        self.events.lock().unwrap().clear();
        *self.messages.lock().unwrap() = vec![self.system_message()];
    }

    pub fn archive(&self) -> Archive {
        Archive {
            backend: self.backend_id().into(),
            legacy_origin: self.legacy_origin.clone(),
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
        *self.charges.lock().unwrap() = crate::backend::Charges::default();
        let limits = self.model_limits(events)?;
        self.emit_context(events)?;
        loop {
            if self.cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            let completion = self.next_completion(limits, events)?;
            let message = completion.message;
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
            if let Some(error) = completion.postprocess_error {
                self.cancel_calls(&completion.calls);
                return Err(error);
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
                &Value::object([
                    (
                        "error",
                        Value::string("Tool was not executed because the turn was interrupted"),
                    ),
                    ("outcome", Value::string("not_executed")),
                ]),
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

    fn projected_estimate(&self, context: &crate::context::Context, messages: &[Value]) -> usize {
        match context.measured_input(messages, self.client.inputs()) {
            Some(tokens) => context.estimate_growth(messages, tokens, self.client.inputs()),
            None => self.unmeasured_estimate(context, messages),
        }
    }

    fn unmeasured_estimate(&self, context: &crate::context::Context, messages: &[Value]) -> usize {
        context.estimate_projected(
            &self.projected_context(context, messages),
            self.client.inputs(),
        )
    }

    fn context_bytes(&self, messages: &[Value]) -> usize {
        crate::context::bytes_for(messages, self.client.inputs())
    }

    fn context_estimate(&self, messages: &[Value]) -> usize {
        match self.context.measured_input(messages, self.client.inputs()) {
            Some(tokens) => self
                .context
                .estimate_growth(messages, tokens, self.client.inputs()),
            None => self
                .unmeasured_estimate(&self.context, messages)
                .saturating_add(self.request_environment().len()),
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

mod backend;
pub(crate) mod checkpoints;
mod compaction;
mod environment;
mod execution;
mod history;
mod input;
mod lifecycle;
mod persistence;
mod project_instructions;
mod recovery;
mod retraction;
mod routing;
mod skills;
mod streaming;
mod summary;
mod temporary;
mod timing;

#[cfg(test)]
mod attachment_tests;
#[cfg(test)]
mod backend_tests;
#[cfg(test)]
mod browser_tests;
#[cfg(test)]
mod checkpoint_tests;
#[cfg(test)]
mod compaction_tests;
#[cfg(test)]
mod context_estimate_tests;
#[cfg(test)]
mod context_status_tests;
#[cfg(test)]
mod context_tests;
#[cfg(test)]
mod delete_tests;
#[cfg(test)]
mod discovery_tests;
#[cfg(test)]
mod history_tests;
#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod instruction_tests;
#[cfg(test)]
mod large_request_tests;
#[cfg(test)]
mod local_read_tests;
#[cfg(test)]
mod long_work_tests;
#[cfg(test)]
mod map_tests;
#[cfg(test)]
mod network_tests;
#[cfg(test)]
mod persistence_tests;
#[cfg(test)]
mod process_tests;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod request_timing_tests;
#[cfg(test)]
mod routing_tests;
#[cfg(test)]
mod skills_tests;
#[cfg(test)]
mod summary_tests;
#[cfg(test)]
mod tests;
