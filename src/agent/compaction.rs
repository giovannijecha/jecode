use super::Agent;
use crate::{
    context,
    events::{Event, EventSink},
    json::Value,
    openrouter::{Completion, FailureKind, Limits, Update},
    sessions::Stage,
};

impl Agent {
    pub(super) fn next_completion(
        &mut self,
        limits: Option<Limits>,
        events: &mut impl EventSink,
    ) -> Result<Completion, String> {
        loop {
            self.compact_if_needed(limits, false, events)?;
            events.emit(Event::Waiting {
                model: self.redact(self.model()),
            })?;
            let messages = self.transport_messages();
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
                    let input = self.context_estimate(&self.messages.lock().unwrap());
                    limits.output_allowance(input)
                })
            });
            if output_tokens == Some(0) {
                return Err("No output space remains in the model context; the original session is preserved".into());
            }
            let persistence = self.persistence.clone();
            let request = crate::attachments::provider::materialize(
                messages.clone(),
                self.tools.attachments(),
                self.client.inputs(),
            )?;
            let result = self.complete_retry(&request, true, output_tokens, &mut |update| {
                match &update {
                    Update::Text(text) => {
                        if let Some(persistence) = &persistence {
                            persistence.partial(text)?;
                        }
                    }
                    Update::Retry { .. } => {
                        if let Some(persistence) = &persistence {
                            persistence.partial("")?;
                        }
                    }
                    _ => {}
                }
                events.emit(self.update_event(update))
            });
            match result {
                Ok(completion) => {
                    let input_end = self.messages.lock().unwrap().len();
                    self.context.observe(completion.usage.as_ref(), input_end);
                    self.context.calibrate(
                        self.context_bytes(&messages)
                            .saturating_add(crate::tools::definitions().encode().len()),
                    );
                    return Ok(completion);
                }
                Err(error) if error.kind == FailureKind::Context => {
                    // Rejected projections cannot retain a density assumption.
                    self.context.calibration = None;
                    if let Some(persistence) = &self.persistence {
                        persistence.partial("")?;
                    }
                    events.emit(Event::RequestDiscarded)?;
                    let estimate = self.context_estimate(&self.messages.lock().unwrap());
                    let ceiling = self
                        .context
                        .ceiling
                        .unwrap_or_else(|| limits.map_or(estimate, |limits| limits.context));
                    self.context.ceiling = Some(ceiling.min(estimate).saturating_mul(3) / 4);
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
        let original = self.messages.lock().unwrap();
        let estimate = self.context_estimate(&original);
        let limits = match limits {
            Some(limits) => limits,
            None if forced || self.context.ceiling.is_some() => Limits {
                context: self.context.ceiling.unwrap_or(estimate).max(1),
                output: None,
            },
            None => return Ok(false),
        };
        let (input_budget, _) = self.context.budgets(limits);
        if !forced && estimate < input_budget {
            return Ok(false);
        }
        if !forced {
            let before = self.context_bytes(&self.projected_context(&self.context, &original));
            let mut preview = self.context.clone();
            preview.preview_until = original.len();
            preview.preview_limit = (input_budget / 12).clamp(512, 2048);
            preview.input_tokens = None;
            preview.measured_end = 0;
            let after = self.context_bytes(&self.projected_context(&preview, &original));
            let large_output = original[self.context.from..].iter().any(|message| {
                message.get("role").and_then(Value::as_str) == Some("tool")
                    && message
                        .get("content")
                        .and_then(Value::as_str)
                        .and_then(|text| crate::json::parse(text).ok())
                        .is_some_and(|result| {
                            ["stdout", "stderr"].iter().any(|key| {
                                result
                                    .get(key)
                                    .and_then(Value::as_str)
                                    .is_some_and(|text| text.len() > preview.preview_limit / 3)
                            })
                        })
            });
            if large_output
                && after < before
                && self
                    .projected_estimate(&preview, &original)
                    .saturating_add(self.request_environment().len())
                    < input_budget
            {
                // Keep every conversation group and its proof. A native preview
                // can remove oversized tool payloads without a model summary.
                drop(original);
                events.emit(Event::Maintenance {
                    text: "Large tool results shown as context previews · full original results remain readable in history".into(),
                })?;
                let previous = std::mem::replace(&mut self.context, preview);
                if let Err(error) = self.checkpoint(Stage::Preserve) {
                    self.context = previous;
                    return Err(error);
                }
                self.record_local_details(
                    "Context previews",
                    "Reduced large tool payloads without summarizing conversation",
                    "notice",
                    &[
                        ("before_bytes".into(), before.to_string()),
                        ("after_bytes".into(), after.to_string()),
                    ],
                );
                return Ok(false);
            }
        }
        let end = original.len();
        if self.context.from >= end {
            return Ok(false);
        }
        // Like a context checkpoint: the summary replaces everything since the
        // previous one, beside recent user requests kept verbatim.
        let mut next = self.context.clone();
        next.from = end;
        next.preview_until = end;
        next.preview_limit = (input_budget / 12).clamp(512, 2048);
        next.request_limit = (input_budget / 4).max(1024);
        next.summary = " ".into();
        let fixed = next.estimate_bytes(
            self.context_bytes(&self.projected_context(&next, &original))
                .saturating_add(crate::tools::definitions().encode().len())
                .saturating_add(self.request_environment().len())
                .saturating_add(768),
        );
        let limit = input_budget.saturating_sub(fixed).min(input_budget / 2);
        if limit < 512 {
            return Err("The original user requests and system instructions exceed the available context. The original session is preserved; choose a model with more context.".into());
        }
        let transcript = original[self.context.from..end]
            .iter()
            .enumerate()
            .map(|(index, message)| {
                (
                    self.context.from + index,
                    context::portable(&next.project_message(
                        message,
                        self.context.from + index,
                        self.compatible_from,
                    ))
                    .encode(),
                )
            })
            .collect::<Vec<_>>();
        let requests = next.user_requests(&original);
        let before_bytes = self.context_bytes(&self.projected_context(&self.context, &original));
        drop(original);
        events.emit(Event::Maintenance {
            text: format!(
                "Compacting context · {} · original conversation kept",
                self.model()
            ),
        })?;
        let (summary, output_floor) =
            self.summarize(&transcript, &requests, limits, limit, events)?;
        next.summary = self.redact(&summary);
        next.summary_output_tokens = output_floor;
        next.input_tokens = None;
        next.measured_end = 0;
        let original = self.messages.lock().unwrap();
        if self
            .projected_estimate(&next, &original)
            .saturating_add(self.request_environment().len())
            >= input_budget
        {
            return Err("The summary and original user requests do not fit the available context; the previous context and original session are preserved".into());
        }
        let after_bytes = self.context_bytes(&self.projected_context(&next, &original));
        drop(original);
        if after_bytes >= before_bytes {
            return Err("Context compaction did not reduce the request. The original conversation and context are preserved; choose a model with more context or retry.".into());
        }
        let count = end - self.context.from;
        let previous = std::mem::replace(&mut self.context, next);
        let text = format!("Context compacted · {count} messages summarized");
        self.record_local_details(
            "Context compaction",
            "Conversation summary prepared",
            "notice",
            &[("model".into(), self.model().into())],
        );
        if let Some(Value::Object(event)) = self.events.lock().unwrap().last_mut() {
            event.insert("summary".into(), Value::string(&self.context.summary));
            event.insert("context_from".into(), Value::number(end));
        }
        if let Err(error) = self.checkpoint(Stage::Preserve) {
            self.context = previous;
            return Err(error);
        }
        events.emit(Event::ContextCompacted { text })?;
        Ok(true)
    }
}

#[cfg(test)]
mod summary_budget_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod preview_tests;
