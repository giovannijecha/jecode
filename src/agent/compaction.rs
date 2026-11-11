use super::Agent;
use crate::{
    backend::{Completion, FailureKind, Limits},
    context,
    events::{Event, EventSink},
    json::Value,
    sessions::Stage,
};

impl Agent {
    /// Applies from the next model request of the active conversation.
    pub fn set_auto_compact(&self, percent: u8) {
        self.auto_compact
            .store(percent, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn auto_compact_percent(&self) -> u8 {
        self.auto_compact.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Only counts actually reported for this provider, model and account.
    pub fn context_status(&self) -> context::Status {
        context::Status {
            tokens: self.context.reported.tokens_for(&self.identity()),
            capacity: self.client.known_limits().map(|limits| limits.context),
            auto_compact: self.auto_compact_percent(),
            pending: None,
        }
    }

    pub(super) fn emit_context(&self, events: &mut impl EventSink) -> Result<(), String> {
        events.emit(Event::Context(self.context_status()))
    }

    /// The current conversation estimate, with collaboration state read
    /// before messages are locked.
    pub(super) fn locked_estimate(&self) -> usize {
        let environment = self.environment();
        self.context_estimate(&environment, &self.messages.lock().unwrap())
    }

    pub(super) fn next_completion(
        &mut self,
        limits: Option<Limits>,
        events: &mut impl EventSink,
    ) -> Result<Completion, String> {
        let mut retries = super::recovery::Retries::default();
        loop {
            events.emit(Event::Preparing)?;
            self.accept_input(events)?;
            // Show the window automatic compaction is about to judge.
            events.emit(Event::Context(
                self.context_status().with_estimate(self.locked_estimate()),
            ))?;
            self.compact_if_needed(limits, false, events)?;
            if self.accept_input(events)? {
                // Input arriving during a summary can change the budget.
                continue;
            }
            let timing = super::timing::RequestTiming::start();
            let seen = self.collaboration.latest_return_sequence();
            let request = self.transport_request();
            events.emit(Event::Context(
                self.context_status()
                    .with_estimate(request.estimated_tokens),
            ))?;
            let messages = &request.messages;
            self.collaboration_seen = seen;
            let output_tokens = limits.and_then(|limits| {
                limits.output.map(|_| {
                    let limits = Limits {
                        context: self
                            .context
                            .ceiling
                            .unwrap_or(limits.context)
                            .min(limits.context),
                        ..limits
                    };
                    limits.output_allowance(request.estimated_tokens)
                })
            });
            if output_tokens == Some(0) {
                return Err("No output space remains in the model context; the original session is preserved".into());
            }
            let persistence = self.persistence.clone();
            let mut emit = |event: Event| {
                match &event {
                    Event::Streaming { text } => {
                        if let Some(persistence) = &persistence {
                            persistence.partial(text)?;
                        }
                    }
                    Event::Recovering { .. } => {
                        if let Some(persistence) = &persistence {
                            persistence.partial("")?;
                        }
                    }
                    _ => {}
                }
                events.emit(event)
            };
            // Rebuild after every retry wait so newly submitted messages
            // reach the next provider request as well.
            let result = self.complete_attempt(
                messages,
                super::recovery::Purpose::Conversation,
                output_tokens,
                &mut retries,
                timing,
                &mut emit,
            );
            match result {
                Ok(None) => {}
                Ok(Some(completion)) => {
                    self.context
                        .record_request(completion.usage.as_ref(), request.measurement);
                    for (before, state) in request.states {
                        self.context.anchor_state(before, state);
                    }
                    self.deliver_completions(request.completions, &mut emit)?;
                    if let Some(event) = completion.usage.as_ref().and_then(generated) {
                        emit(event)?;
                    }
                    return Ok(completion);
                }
                Err(error) if error.kind == FailureKind::Context => {
                    retries = super::recovery::Retries::default();
                    // Rejected projections cannot retain a density assumption.
                    self.context.calibration = None;
                    if let Some(persistence) = &self.persistence {
                        persistence.partial("")?;
                    }
                    events.emit(Event::RequestDiscarded)?;
                    let estimate = self.locked_estimate();
                    let ceiling = self
                        .context
                        .ceiling
                        .unwrap_or_else(|| limits.map_or(estimate, |limits| limits.context));
                    self.context.ceiling = Some(ceiling.min(estimate).saturating_mul(3) / 4);
                    self.emit_context(events)?;
                    if !self.compact_if_needed(limits, true, events)? {
                        return Err(format!(
                            "{} The context could not be reduced; the original session is preserved.",
                            error.message
                        ));
                    }
                }
                Err(error) if error.kind == FailureKind::Length => {
                    let allowance = output_tokens.map_or_else(
                        || "provider default".to_owned(),
                        |tokens| format!("{tokens} tokens"),
                    );
                    return Err(format!(
                        "{} (model: {}; effort: {}; output allowance: {allowance}). Completed tool results and the original session are preserved.",
                        error.message,
                        self.model(),
                        self.effort().name()
                    ));
                }
                Err(error) => return Err(error.message),
            }
        }
    }

    fn compact_if_needed(
        &mut self,
        limits: Option<Limits>,
        forced: bool,
        events: &mut impl EventSink,
    ) -> Result<bool, String> {
        let environment = self.environment();
        let original = self.messages.lock().unwrap();
        let estimate = self.context_estimate(&environment, &original);
        let limits = match limits {
            Some(limits) => limits,
            None if forced || self.context.ceiling.is_some() => Limits {
                context: self.context.ceiling.unwrap_or(estimate).max(1),
                output: None,
            },
            None => return Ok(false),
        };
        // One configured budget triggers reduction, sizes previews and the
        // retained view, and checks the result. Provider rejections force it.
        let percent = self.auto_compact_percent();
        let input_budget = self.context.budget(limits, percent);
        let budget_text = format!(
            "the auto-compact budget ({}% of {} tokens)",
            percent,
            self.context
                .capacity(Some(limits))
                .unwrap_or(limits.context)
        );
        // One mechanism: at the budget, or on a provider rejection, the same
        // model summarizes the older prefix. Until then requests only append.
        if !forced && estimate < input_budget {
            return Ok(false);
        }
        if self.context.from >= original.len() {
            return Ok(false);
        }
        let end = self.recent_boundary(&original, input_budget / 4);
        // Summarize the older prefix and keep recent complete exchanges verbatim.
        let mut next = self.context.clone();
        next.clear_measurement();
        next.from = end;
        // State before the boundary is summarized away; current state is sent
        // again. The last summarized instruction snapshot stays as a record
        // outside the window, so the next request restates current
        // instructions even when none apply.
        let summarized = next
            .states
            .iter()
            .rfind(|state| state.before < end && state.is_snapshot())
            .cloned();
        next.states.retain(|state| state.before >= end);
        if next.latest_state(true).is_none() {
            next.states.splice(0..0, summarized);
        }
        next.skill_references = self.redact(&self.skill_checkpoint_instructions());
        let preview_limit = (input_budget / 12).clamp(512, 2048);
        next.request_limit = (input_budget / 4).max(1024);
        next.summary = " ".into();
        let fixed = self
            .context_estimate_for(&next, &environment, &original)
            .saturating_add(next.estimate_bytes(768));
        // A handoff needs room for the active intent, evidence references and
        // next steps, not a fraction of a million-token input window. This is
        // a request to the model, never a post-hoc truncation of its answer.
        let available_tokens = input_budget.saturating_sub(fixed).min(input_budget / 2);
        let limit = next.summary_byte_budget(available_tokens).min(16 * 1024);
        if limit < 512 {
            return Err(format!(
                "The original user requests and system instructions leave no room for a summary within {budget_text}. The original session and previous context are preserved; raise Auto-compact at in /settings or choose a model with more context."
            ));
        }
        let mut transcript = original[self.context.from..end]
            .iter()
            .enumerate()
            .filter(|(index, _)| !self.context.returned.contains(&(self.context.from + index)))
            .map(|(index, message)| {
                let at = self.context.from + index;
                let projected = if message.get("role").and_then(Value::as_str) == Some("tool") {
                    context::preview(message, at, preview_limit)
                } else {
                    next.project_message(message, at, self.compatible_from)
                };
                (at, context::portable(&projected).encode())
            })
            .collect::<Vec<_>>();
        // Outcomes reported only in runtime state leave the window here
        // without being sent again, so portion summaries read them in place.
        for state in self
            .context
            .states
            .iter()
            .filter(|s| s.before < end && !s.is_snapshot())
        {
            if let Some(section) = super::projection::process_section(&state.text) {
                let at = transcript.partition_point(|(index, _)| *index < state.before);
                transcript.insert(at, (state.before, section.to_owned()));
            }
        }
        let requests = next.user_requests(&original);
        let before_bytes =
            self.context_bytes(&self.instructed_context(&self.context, &environment, &original));
        drop(original);
        events.emit(Event::Maintenance {
            text: format!(
                "Compacting context · {} · original conversation kept",
                self.model()
            ),
        })?;
        // Proactive compaction first asks over the cached conversation itself;
        // a rejected or oversized request summarizes portions instead.
        let in_place = if forced {
            None
        } else {
            let request = self.transport_request();
            self.summarize_in_place(&request, end, limits, limit, events)?
        };
        let (summary, output_floor) = match in_place {
            Some(summary) => summary,
            None => self.summarize(&transcript, &requests, limits, limit, events)?,
        };
        next.summary = self.redact(&summary);
        next.summary_output_tokens = output_floor;
        let environment = self.environment();
        let original = self.messages.lock().unwrap();
        if self.context_estimate_for(&next, &environment, &original) >= input_budget {
            return Err(format!(
                "The summary and original user requests do not fit {budget_text}; the previous context and original session are preserved. Raise Auto-compact at in /settings or choose a model with more context."
            ));
        }
        let after_bytes =
            self.context_bytes(&self.instructed_context(&next, &environment, &original));
        drop(original);
        if after_bytes >= before_bytes {
            return Err("Context compaction did not reduce the request. The original conversation and context are preserved; choose a model with more context or retry.".into());
        }
        let count = end - self.context.from;
        let previous = std::mem::replace(&mut self.context, next);
        let text = format!("Context compacted · {count} messages summarized");
        let mut event = self.local_details_event(
            "Context compaction",
            "Conversation summary prepared",
            "notice",
            &[("model".into(), self.model().into())],
        );
        if let Value::Object(fields) = &mut event {
            fields.insert("summary".into(), Value::string(&self.context.summary));
            fields.insert("context_from".into(), Value::number(end));
        }
        self.events.lock().unwrap().push(event.clone());
        if let Err(error) = self.checkpoint(Stage::Compaction) {
            self.context = previous;
            self.withdraw_event(&event);
            return Err(error);
        }
        events.emit(Event::ContextCompacted { text })?;
        Ok(true)
    }

    pub(super) fn recent_boundary(&self, messages: &[Value], budget: usize) -> usize {
        let mut boundary = messages.len();
        let mut bytes = 0usize;
        let mut measured = 0usize;
        let mut media = 0usize;
        let origin = self.identity();
        let active_start = crate::attachments::view_start(messages, self.client.inputs());
        for index in (self.context.from..messages.len()).rev() {
            if self.context.returned.contains(&index) {
                continue;
            }
            let message = &messages[index];
            if let Some(tokens) = crate::conversation::output_tokens_for(message, &origin) {
                measured = measured.saturating_add(tokens);
            } else {
                let weight = if index >= active_start
                    || message.get("role").and_then(Value::as_str) != Some("tool")
                {
                    crate::attachments::weight_for(message, self.client.inputs())
                } else {
                    0
                };
                let size = context::bytes_for_message(
                    message,
                    index >= active_start,
                    self.client.inputs(),
                );
                bytes = bytes.saturating_add(size.saturating_sub(weight));
                media = media.saturating_add(weight);
            }
            if self
                .context
                .estimate_bytes(bytes)
                .saturating_add(measured)
                .saturating_add(media)
                > budget
            {
                break;
            }
            // A tool call and all its results must stay on the same side.
            if messages[index].get("role").and_then(Value::as_str) != Some("tool") {
                boundary = index;
            }
        }
        if boundary == self.context.from {
            // There is no older prefix to replace in this case.
            messages.len()
        } else {
            boundary
        }
    }
}

/// The turn's reported input and output for one completed request.
pub(super) fn generated(usage: &crate::backend::Usage) -> Option<Event> {
    (usage.input_tokens.is_some() || usage.output_tokens.is_some()).then(|| Event::Generated {
        input: usage.input_tokens.unwrap_or(0),
        tokens: usage.output_tokens.unwrap_or(0),
    })
}

#[cfg(test)]
mod in_place_tests;
#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod summary_budget_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod thinking_tests;

#[cfg(test)]
mod continuity_tests;
#[cfg(all(test, windows))]
mod persistence_tests;
