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
            let result = self.complete_retry(&messages, true, output_tokens, &mut |update| {
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
                    let (input_end, review_bytes) = {
                        let original = self.messages.lock().unwrap();
                        (
                            original.len(),
                            self.completion_review_state(&original).len(),
                        )
                    };
                    self.measured_review_bytes = review_bytes;
                    self.context.observe(completion.usage.as_ref(), input_end);
                    self.context.calibrate(
                        context::bytes(&messages)
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
            let before = context::bytes(&self.projected_context(&self.context, &original));
            let mut preview = self.context.clone();
            preview.preview_until = original.len();
            preview.preview_limit = (input_budget / 12).clamp(512, 2048);
            preview.input_tokens = None;
            preview.measured_end = 0;
            let after = context::bytes(&self.projected_context(&preview, &original));
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
                    .saturating_add(self.request_environment(&original).len())
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
                    "Reduced large tool payloads without summarizing conversation or changing saved proof",
                    "notice",
                    &[("before_bytes".into(), before.to_string()), ("after_bytes".into(), after.to_string())],
                );
                return Ok(false);
            }
        }
        let groups = context::groups(&original, self.context.from);
        let end = original.len();
        let mut next = self.context.clone();
        next.preview_until = end;
        next.preview_limit = (input_budget / 12).clamp(512, 2048);
        let mut recent = 0usize;
        let mut cut = end;
        for &(start, end) in groups.iter().rev() {
            let size = original[start..end]
                .iter()
                .enumerate()
                .map(|(index, message)| {
                    next.project_message(message, start + index, self.compatible_from)
                        .encode()
                        .len()
                })
                .sum();
            if recent > 0 && recent.saturating_add(size) > input_budget / 3 {
                break;
            }
            recent = recent.saturating_add(size);
            cut = start;
        }
        if cut == self.context.from && !groups.is_empty() {
            cut = groups[0].1;
        }
        if cut == self.context.from && self.context.summary.is_empty() {
            return Ok(false);
        }
        let preferred_cut = cut;
        next.from = cut;
        // Include the progress header and owned evidence when reserving memory space.
        next.summary = " ".into();
        let fixed_cost = |candidate: &context::Context| {
            candidate.estimate_bytes(
                context::bytes(&self.projected_context(candidate, &original))
                    .saturating_add(crate::tools::definitions().encode().len())
                    .saturating_add(self.request_environment(&original).len())
                    .saturating_add(768),
            )
        };
        // Reserve room for carried facts and growth instead of imposing an 8 KiB job ceiling.
        let memory_reserve = (input_budget / 2).min(
            8192.max(
                self.context
                    .memory_view()
                    .len()
                    .saturating_add(input_budget / 16),
            ),
        );
        while fixed_cost(&next).saturating_add(memory_reserve) > input_budget && cut < end {
            // Prioritize carried work over older recent groups, retaining original references.
            cut = context::groups(&original, cut)[0].1;
            next.from = cut;
        }
        let fixed = fixed_cost(&next);
        let memory_limit = input_budget.saturating_sub(fixed).min(input_budget / 2);
        if memory_limit < 512 {
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
        let before_bytes = context::bytes(&self.projected_context(&self.context, &original));
        drop(original);
        events.emit(Event::Maintenance {
            text: format!(
                "Compacting context · {} · original conversation kept",
                self.model()
            ),
        })?;
        let (summary, output_floor) =
            self.summarize(&transcript, &requests, limits, memory_limit, events)?;
        next.summary = self.redact(&summary);
        next.summary_output_tokens = output_floor;
        next.memory_limit = Some(memory_limit);
        next.input_tokens = None;
        next.measured_end = 0;
        let original = self.messages.lock().unwrap();
        while self
            .projected_estimate(&next, &original)
            .saturating_add(self.request_environment(&original).len())
            >= input_budget
            && cut < end
        {
            cut = context::groups(&original, cut)[0].1;
            next.from = cut;
        }
        if self
            .projected_estimate(&next, &original)
            .saturating_add(self.request_environment(&original).len())
            >= input_budget
        {
            return Err("Continuity memory and original user requests do not fit the available context; the previous context and original session are preserved".into());
        }
        // The actual memory may be much smaller than its growth reservation.
        // Reclaim recent complete groups when both capacity and reduction permit it.
        let reclaimed = context::groups(&original, preferred_cut)
            .into_iter()
            .filter(|(_, end)| *end <= cut)
            .collect::<Vec<_>>();
        for (start, _) in reclaimed.into_iter().rev() {
            next.from = start;
            if self
                .projected_estimate(&next, &original)
                .saturating_add(self.request_environment(&original).len())
                >= input_budget
                || context::bytes(&self.projected_context(&next, &original)) >= before_bytes
            {
                next.from = cut;
                break;
            }
            cut = start;
        }
        let after_bytes = context::bytes(&self.projected_context(&next, &original));
        drop(original);
        if after_bytes >= before_bytes {
            return Err("Context compaction did not reduce the request. The original conversation and context are preserved; choose a model with more context or retry.".into());
        }
        let count = cut - self.context.from;
        let previous = std::mem::replace(&mut self.context, next);
        let text = format!(
            "Context compacted · {count} messages summarized · {} kept",
            end - cut
        );
        self.record_local_details(
            "Context compaction",
            "Continuity summary prepared",
            "notice",
            &[("model".into(), self.model().into())],
        );
        if let Some(Value::Object(event)) = self.events.lock().unwrap().last_mut() {
            event.insert("summary".into(), Value::string(&self.context.summary));
            event.insert("context_from".into(), Value::number(cut));
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
