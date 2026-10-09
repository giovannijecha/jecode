use super::Agent;
use crate::{
    context::memory,
    events::{Event, EventSink},
    json::Value,
    openrouter::{FailureKind, Limits, Update},
};

impl Agent {
    pub(super) fn summarize(
        &self,
        records: &[(usize, String)],
        requests: &str,
        limits: Limits,
        memory_limit: usize,
        events: &mut impl EventSink,
    ) -> Result<(String, usize), String> {
        let capacity = self
            .context
            .ceiling
            .unwrap_or(limits.context)
            .min(limits.context);
        let mut summary = self.context.summary.clone();
        let prompt = memory::prompt();
        // Previews shorten display content; only accepted summaries age proof.
        let evidence_since = self.context.from;
        let mut position = (0usize, 0usize);
        let mut output_tokens = limits.output.unwrap_or_else(|| {
            (capacity / 8)
                .max(self.context.summary_output_tokens)
                .max(1)
        });
        let mut output_floor = self.context.summary_output_tokens;
        loop {
            if self.cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            let prefix = format!(
                "Original user requests:\n{requests}\n\n{}{}Previous continuity memory:\n{}\n\nReturn at most {memory_limit} UTF-8 bytes. New original transcript:\n",
                self.delivered_report_hint(),
                memory::request_review_hint(
                    &summary,
                    &self.messages.lock().unwrap(),
                    evidence_since
                ),
                memory::prompt_view(&memory::summary_input(&summary, memory_limit)),
            );
            let minimal_portion =
                records
                    .get(position.0)
                    .map_or_else(String::new, |(index, body)| {
                        // Leave room for a useful source portion, not just its label.
                        let remaining = &body[position.1..];
                        let mut take = remaining.len().min(memory_limit);
                        while !remaining.is_char_boundary(take) {
                            take -= 1;
                        }
                        format!(
                            "{}{}",
                            source_label(*index, position.1, &self.messages.lock().unwrap()),
                            &remaining[..take]
                        )
                    });
            let minimal_request = [
                Value::object([
                    ("role", Value::string("system")),
                    ("content", Value::string(&prompt)),
                ]),
                Value::object([
                    ("role", Value::string("user")),
                    (
                        "content",
                        Value::string(format!("{prefix}{minimal_portion}")),
                    ),
                ]),
            ];
            let fixed_input = self
                .context
                .estimate_bytes(crate::context::bytes(&minimal_request).saturating_add(768));
            let output_ceiling = Limits {
                context: capacity,
                ..limits
            }
            .output_allowance(fixed_input);
            if output_ceiling == 0 {
                return Err("Insufficient context for continuity input and output; the original session is preserved".into());
            }
            output_tokens = output_tokens.min(output_ceiling);
            let mut budget = self
                .context
                .summary_byte_budget(capacity.saturating_sub(output_tokens))
                .saturating_sub(prompt.len() + prefix.len() + 768);
            if budget < 128 {
                return Err("Insufficient context to prepare continuity memory; the original session is preserved".into());
            }
            let mut repair = String::new();
            let mut repair_portion = None;
            let mut invalid = 0;
            let next;
            loop {
                let (portion, end) = match &repair_portion {
                    Some((sources, end)) => (String::clone(sources), *end),
                    None => portion(records, &self.messages.lock().unwrap(), position, budget)?,
                };
                let messages = [
                    Value::object([
                        ("role", Value::string("system")),
                        ("content", Value::string(&prompt)),
                    ]),
                    Value::object([
                        ("role", Value::string("user")),
                        (
                            "content",
                            Value::string(format!("{prefix}{portion}{repair}")),
                        ),
                    ]),
                ];
                // Budget the serialized request too: transcript JSON is escaped again
                // inside the summary message, and a repair adds its own payload.
                let request_bytes = crate::context::bytes(&messages).saturating_add(768);
                let available = self
                    .context
                    .summary_byte_budget(capacity.saturating_sub(output_tokens));
                if request_bytes > available {
                    if repair_portion.take().is_some() {
                        // Oversized proof facts fall back to the original bounded
                        // transcript repair; the previous context remains intact.
                        continue;
                    }
                    let smaller = budget.saturating_sub(request_bytes - available).max(128);
                    if smaller >= budget {
                        return Err("Insufficient context for the serialized continuity request; the original session is preserved".into());
                    }
                    budget = smaller;
                    continue;
                }
                match self.complete_retry(&messages, false, Some(output_tokens), &mut |update| {
                    events.emit(match update {
                        Update::Text(_) => Event::Working,
                        update => self.update_event(update),
                    })
                }) {
                    Ok(completion) => {
                        let candidate = self.redact(&completion.text);
                        let checked = if completion.calls.is_empty() {
                            memory::prepare_with_notices(
                                &candidate,
                                &summary,
                                &self.messages.lock().unwrap(),
                                evidence_since,
                                memory_limit,
                            )
                        } else {
                            Err("Summary tools must be disabled".into())
                        };
                        match checked {
                            Ok((valid, notices)) => {
                                if !notices.is_empty() {
                                    self.record_local_details(
                                        "Context memory reconciliation",
                                        "Unknown prior-work references did not resolve or withdraw items; original work is retained for review",
                                        "warning", &[("unresolved_labels".into(), notices.join(", "))],
                                    );
                                    events.emit(Event::Maintenance {
                                        text: "Retaining unresolved continuity work for review · original items preserved".into(),
                                    })?;
                                }
                                summary = valid;
                                next = end;
                                break;
                            }
                            Err(error) => {
                                invalid += 1;
                                self.record_local_details(
                                    "Context memory validation",
                                    &error,
                                    "warning",
                                    &[
                                        ("rejected_memory".into(), candidate.clone()),
                                        ("previous_memory".into(), summary.clone()),
                                        ("evidence_since".into(), evidence_since.to_string()),
                                        ("memory_limit".into(), memory_limit.to_string()),
                                    ],
                                );
                                if invalid >= 2 {
                                    return Err(format!(
                                        "Continuity memory could not be validated. The previous context and original session are preserved. {}",
                                        self.redact(&error)
                                    ));
                                }
                                events.emit(Event::Maintenance {
                                    text:
                                        "Repairing continuity memory · original context preserved"
                                            .into(),
                                })?;
                                repair = memory::validation_feedback(
                                    &candidate,
                                    &error,
                                    &summary,
                                    &self.messages.lock().unwrap(),
                                    evidence_since,
                                    memory_limit,
                                );
                                if memory::repair_has_update_facts(&candidate, memory_limit) {
                                    let last = end.0 + usize::from(end.1 > 0);
                                    repair_portion = Some((
                                        memory::repair_sources(
                                            &self.messages.lock().unwrap(),
                                            records[position.0..last]
                                                .iter()
                                                .map(|(index, _)| *index),
                                        ),
                                        end,
                                    ));
                                } else {
                                    // A missing update still needs the transcript;
                                    // native proof facts cannot supply its task facts.
                                    repair_portion = None;
                                }
                                budget = budget.saturating_sub(repair.len()).max(128);
                            }
                        }
                    }
                    Err(error) if error.kind == FailureKind::Context && budget > 128 => {
                        repair_portion = None;
                        budget /= 2;
                    }
                    Err(error) if error.kind == FailureKind::Length => {
                        let larger = output_tokens.saturating_mul(2).min(output_ceiling);
                        if larger <= output_tokens {
                            return Err(format!(
                                "Continuity memory was truncated; original session preserved. {}",
                                error.message
                            ));
                        }
                        output_tokens = larger;
                        output_floor = output_floor.max(larger);
                        budget = self
                            .context
                            .summary_byte_budget(capacity.saturating_sub(output_tokens))
                            .saturating_sub(prompt.len() + prefix.len() + repair.len() + 768)
                            .min(budget);
                        if budget < 128 {
                            return Err("Insufficient context for a complete continuity memory; original session preserved".into());
                        }
                    }
                    Err(error) => return Err(error.message),
                }
            }
            position = next;
            if position.0 >= records.len() {
                return Ok((summary, output_floor));
            }
        }
    }
}

#[cfg(test)]
mod repair_tests;

// Whole records stay together where possible; a large record always retains its source label.
fn portion(
    records: &[(usize, String)],
    messages: &[Value],
    start: (usize, usize),
    budget: usize,
) -> Result<(String, (usize, usize)), String> {
    let (mut record, mut offset) = start;
    let mut text = String::new();
    while let Some((index, body)) = records.get(record) {
        let label = source_label(*index, offset, messages);
        let available = budget.saturating_sub(text.len() + label.len());
        if available == 0 {
            break;
        }
        let remaining = &body[offset..];
        if remaining.len() > available && !text.is_empty() {
            break;
        }
        let mut take = remaining.len().min(available);
        while !remaining.is_char_boundary(take) {
            take -= 1;
        }
        if take == 0 {
            break;
        }
        text.push_str(&label);
        text.push_str(&remaining[..take]);
        offset += take;
        if offset == body.len() {
            record += 1;
            offset = 0;
        } else {
            break;
        }
    }
    if record < records.len() && (record, offset) == start {
        return Err(
            "Insufficient context for the next original transcript portion; source preserved"
                .into(),
        );
    }
    Ok((text, (record, offset)))
}

fn source_label(index: usize, offset: usize, messages: &[Value]) -> String {
    format!(
        "\nNative source: {}\nOriginal message, byte {offset}:\n",
        memory::source_record(messages, index).encode(),
    )
}
