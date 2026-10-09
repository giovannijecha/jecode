use super::Agent;
use crate::{
    context::{excerpt, source},
    events::{Event, EventSink},
    json::Value,
    openrouter::{FailureKind, Limits, Update},
};

const PROMPT: &str = "You are performing a context checkpoint compaction. Write a handoff summary for another language model that will resume this coding task in a fresh context window. Fold the transcript into the previous summary and include:
- current progress and key decisions, with exact file paths, commands and results
- important context, constraints and user preferences
- errors still open and what remains to be done, with clear next steps
- critical data, examples or references needed to continue
Every original message remains readable through the read tool as history:N, so cite those references instead of copying long content. Be concise and structured, and reply with the summary only.";

impl Agent {
    /// Summarizes the transcript in one request and returns the summary with the
    /// output allowance it needed. When the transcript does not fit, its oldest
    /// records are left out; they remain readable as history.
    pub(super) fn summarize(
        &self,
        records: &[(usize, String)],
        requests: &str,
        limits: Limits,
        limit: usize,
        events: &mut impl EventSink,
    ) -> Result<(String, usize), String> {
        let capacity = self
            .context
            .ceiling
            .unwrap_or(limits.context)
            .min(limits.context);
        let previous = if self.context.summary.is_empty() {
            "(none)"
        } else {
            &self.context.summary
        };
        let prefix = format!(
            "Original user requests:\n{requests}\n\nPrevious summary:\n{previous}\n\nKeep the updated summary under {limit} bytes.\n\nTranscript since the previous summary:\n"
        );
        let request = |transcript: &str| {
            [
                Value::object([
                    ("role", Value::string("system")),
                    ("content", Value::string(PROMPT)),
                ]),
                Value::object([
                    ("role", Value::string("user")),
                    ("content", Value::string(format!("{prefix}{transcript}"))),
                ]),
            ]
        };
        let fixed_input = self
            .context
            .estimate_bytes(crate::context::bytes(&request("")).saturating_add(768));
        // Keep a quarter of the window for transcript input so a large
        // learned output allowance cannot starve it.
        let output_ceiling = Limits {
            context: capacity,
            ..limits
        }
        .output_allowance(fixed_input.saturating_add(capacity / 4));
        if output_ceiling == 0 {
            return Err("Insufficient context to summarize the conversation; the original session is preserved".into());
        }
        let mut output_tokens = limits
            .output
            .unwrap_or_else(|| {
                (capacity / 8)
                    .max(self.context.summary_output_tokens)
                    .max(1)
            })
            .min(output_ceiling);
        let mut output_floor = self.context.summary_output_tokens;
        let budget_for = |output_tokens: usize| {
            self.context
                .summary_byte_budget(capacity.saturating_sub(output_tokens))
                .saturating_sub(PROMPT.len() + prefix.len() + 768)
        };
        let mut budget = budget_for(output_tokens);
        loop {
            if self.cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            if budget < 128 {
                return Err("Insufficient context to summarize the conversation; the original session is preserved".into());
            }
            let transcript = tail(records, &self.messages.lock().unwrap(), budget);
            match self.complete_retry(
                &request(&transcript),
                false,
                Some(output_tokens),
                &mut |update| {
                    events.emit(match update {
                        Update::Text(_) => Event::Working,
                        update => self.update_event(update),
                    })
                },
            ) {
                Ok(completion) => {
                    let summary = self.redact(completion.text.trim());
                    if summary.is_empty() {
                        return Err("The model returned an empty summary; the original session is preserved".into());
                    }
                    return Ok((summary, output_floor));
                }
                Err(error) if error.kind == FailureKind::Context => budget /= 2,
                Err(error) if error.kind == FailureKind::Length => {
                    let larger = output_tokens.saturating_mul(2).min(output_ceiling);
                    if larger <= output_tokens {
                        return Err(format!(
                            "The summary was truncated; the original session is preserved. {}",
                            error.message
                        ));
                    }
                    output_tokens = larger;
                    output_floor = output_floor.max(larger);
                    budget = budget_for(output_tokens).min(budget);
                }
                Err(error) => return Err(error.message),
            }
        }
    }
}

/// The newest records that fit `budget`, in order. A newest record larger than
/// the budget is shortened rather than dropped.
fn tail(records: &[(usize, String)], messages: &[Value], budget: usize) -> String {
    const OMITTED: &str =
        "[Older messages are omitted for space; read them through history:N if needed.]\n";
    let budget = budget.saturating_sub(OMITTED.len());
    let mut parts = Vec::new();
    let mut used = 0;
    for (index, body) in records.iter().rev() {
        let label = format!("\nSource: {}\n", source::record(messages, *index).encode());
        let available = budget.saturating_sub(used + label.len());
        if body.len() <= available {
            used += label.len() + body.len();
            parts.push(format!("{label}{body}"));
            continue;
        }
        if parts.is_empty() && available > 0 {
            parts.push(format!("{label}{}", excerpt(body, available)));
        }
        break;
    }
    let omitted = if parts.len() < records.len() {
        OMITTED
    } else {
        ""
    };
    parts.reverse();
    format!("{omitted}{}", parts.concat())
}
